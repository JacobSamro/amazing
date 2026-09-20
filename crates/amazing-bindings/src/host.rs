//! Runtime-owned values and installation hooks; this crate never imports JS.
use amazing_dom::{Dom, DomError, NodeIdx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interface {
    Node,
    Document,
    DocumentFragment,
    DocumentType,
    Element,
    CharacterData,
    Text,
    Comment,
    ProcessingInstruction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    TextConstructor,
    CommentConstructor,
    FragmentConstructor,
    DocumentConstructor,
    CreateText,
    CreateComment,
    CreateFragment,
    Append,
    Prepend,
    ReplaceChildren,
    Before,
    After,
    ReplaceWith,
    Remove,
    AppendChild,
    InsertBefore,
    RemoveChild,
    ReplaceChild,
    CloneNode,
    NodeType,
    NodeName,
    NodeValue,
    SetNodeValue,
    TextContent,
    SetTextContent,
    ParentNode,
    ParentElement,
    OwnerDocument,
    FirstChild,
    LastChild,
    PreviousSibling,
    NextSibling,
    ChildNodes,
    HasChildNodes,
    IsConnected,
    Contains,
    Data,
    SetData,
    Length,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method { length: u8 },
    Getter,
    Setter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: &'static str,
    pub operation: Operation,
    pub kind: MemberKind,
}

/// Runtime registers standard descriptors/prototypes and forwards native calls
/// to `invoke`. None means an illegal constructor, not a missing global.
#[derive(Clone, Copy, Debug)]
pub struct InterfaceDefinition {
    pub interface: Interface,
    pub name: &'static str,
    pub parent: Option<Interface>,
    pub constructor: Option<Operation>,
    pub members: &'static [Member],
}

/// The JS runtime implements this trait. The driver supplies access to its same
/// parsed arena. No user code may run inside `with_dom`; conversions and value
/// wrapping happen outside its borrow. Error values preserve JS exceptions.
///
/// Node brands and wrapper caches must be engine-private (not writable JS
/// properties). `wrap_node` is canonical per arena/realm, including document.
/// Legacy DOM factories and query results must use that same wrapper cache.
pub trait BindingHost: Sized {
    type Value: Clone;
    type Error;

    fn install_interface(
        &mut self,
        definition: &'static InterfaceDefinition,
    ) -> Result<(), Self::Error>;
    /// Standard Node constants are exposed on Node and its prototype.
    fn install_node_constant(&mut self, name: &'static str, value: u16) -> Result<(), Self::Error>;
    fn current_document(&self) -> NodeIdx;
    fn with_dom<R>(
        &mut self,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>,
    ) -> Result<R, Self::Error>;
    /// Non-node values return None without invoking user code.
    fn node_brand(&self, value: &Self::Value) -> Option<NodeIdx>;
    fn wrap_node(&mut self, node: NodeIdx) -> Result<Self::Value, Self::Error>;
    /// Live, same-object NodeList; indexed access, length and item read the arena.
    fn child_nodes(&mut self, parent: NodeIdx) -> Result<Self::Value, Self::Error>;
    fn is_html_document(&self, document: NodeIdx) -> bool;

    fn undefined(&self) -> Self::Value;
    fn null(&self) -> Self::Value;
    fn boolean(&self, value: bool) -> Self::Value;
    fn number(&self, value: f64) -> Self::Value;
    fn string(&self, value: &[u16]) -> Self::Value;
    fn is_undefined(&self, value: &Self::Value) -> bool;
    fn is_nullish(&self, value: &Self::Value) -> bool;
    fn to_boolean(&self, value: &Self::Value) -> bool;
    /// WebIDL DOMString conversion, preserving code units and thrown coercions.
    fn to_dom_string(&mut self, value: &Self::Value) -> Result<Vec<u16>, Self::Error>;
    fn type_error(&mut self, message: &'static str) -> Self::Error;
}
