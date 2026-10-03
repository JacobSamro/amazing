//! CSS Values5 tree-counting and Shadow1 §4.2 defining-tree provenance.
//! Metadata is private to substitution; the public syntax Token stays unchanged.
use super::*;
use amazing_dom::{Dom,NodeIdx};

pub(crate) fn contains(css:&str)->bool {
    tokenize(css).iter().any(|token|matches!(token,Token::Function(name) if
        name.eq_ignore_ascii_case("sibling-index")||name.eq_ignore_ascii_case("sibling-count")))
}

#[derive(Debug,Clone,PartialEq)]
pub(super) struct ScopedTokens {
    tokens:Vec<Token>, scopes:Vec<Option<NodeIdx>>,
}
impl std::ops::Deref for ScopedTokens {
    type Target=[Token];fn deref(&self)->&[Token] {&self.tokens}
}
impl ScopedTokens {
    pub(super) fn new(tokens:Vec<Token>,scope:Option<NodeIdx>)->Self {
        Self{scopes:vec![scope;tokens.len()],tokens}
    }
    fn slice(&self,start:usize,end:usize)->Self {
        Self{tokens:self.tokens[start..end].to_vec(),scopes:self.scopes[start..end].to_vec()}
    }
    fn extend(&mut self,other:Self) {self.tokens.extend(other.tokens);self.scopes.extend(other.scopes);}
}
fn tree_root(dom:&Dom,node:NodeIdx)->Option<NodeIdx> {
    let mut current=node;let mut seen=BTreeSet::new();
    loop {
        if !seen.insert(current) {return None;}
        if matches!(dom.get(current).kind,amazing_dom::NodeKind::Document)||dom.is_shadow_root(current) {return Some(current);}
        current=dom.get(current).parent?;
    }
}
pub(crate) struct TreeContext<'a> {
    pub(crate) dom:&'a Dom,pub(crate) node:NodeIdx,
    pub(crate) scopes:BTreeMap<String,Option<NodeIdx>>,
}
impl TreeContext<'_> {
    pub(super) fn scope(&self,name:&str)->Option<NodeIdx> {self.scopes.get(name).copied().flatten()}
    fn allowed(&self,scope:Option<NodeIdx>)->bool {
        let Some(mut scope)=scope else{return false;};
        let target=tree_root(self.dom,self.node);
        let mut seen=BTreeSet::new();
        loop {
            if Some(scope)==target {return true;}
            if !seen.insert(scope) {return false;}
            let Some(shadow)=self.dom.shadow_data(scope) else{return false;};
            let Some(parent)=tree_root(self.dom,shadow.host) else{return false;};
            scope=parent;
        }
    }
    fn resolve(&self,property:&str,tokens:ScopedTokens)->Option<Vec<Token>> {
        let siblings:Vec<_>=self.dom.get(self.node).parent.map(|parent|
            self.dom.children(parent).filter(|n|self.dom.name(*n).is_some()).collect()).unwrap_or_else(||vec![self.node]);
        let index=siblings.iter().position(|n|*n==self.node)?+1;
        let mut output=Vec::new();let mut i=0;
        let mut functions:Vec<Option<String>>=Vec::new();let mut pending=None;
        while i<tokens.len() {
            let count=match &tokens[i] {
                Token::Function(name) if name.eq_ignore_ascii_case("sibling-index")=>Some(index),
                Token::Function(name) if name.eq_ignore_ascii_case("sibling-count")=>Some(siblings.len()),_=>None,
            };
            if let Some(count)=count {
                if tokens.get(i+1)!=Some(&Token::OpenParen) {return None;}
                let mut end=i+2;while tokens.get(end)==Some(&Token::Whitespace) {end+=1;}
                if tokens.get(end)!=Some(&Token::CloseParen) {return None;}
                let value=if self.allowed(tokens.scopes[i]) {count as f64}else{0.0};
                if numeric_slot(property,functions.last().and_then(|name|name.as_deref())) {
                    output.push(Token::Number(value));
                } else {
                    // A computed numeric function is still a number. Literal
                    // zero alone would accidentally gain CSS's special
                    // unitless-zero length/angle grammar. calc() retains type.
                    output.extend([Token::Function("calc".into()),Token::OpenParen,Token::Number(value),Token::CloseParen]);
                }
                i=end+1;
            } else {
                match &tokens[i] {
                    Token::Function(name)=>pending=Some(name.to_ascii_lowercase()),
                    Token::OpenParen=>functions.push(pending.take()),
                    Token::CloseParen=>{functions.pop();},_=>{},
                }
                output.push(tokens[i].clone());i+=1;
            }
        }
        Some(output)
    }
}
fn numeric_slot(property:&str,parent:Option<&str>)->bool {
    match parent {
        Some(name)=>matches!(name,"calc"|"min"|"max"|"clamp"|"sign"|"round"|"mod"|"rem"|
            "abs"|"pow"|"sqrt"|"hypot"|"log"|"exp"|"sin"|"cos"|"tan"|"asin"|"acos"|"atan"|"atan2"|
            "rgb"|"rgba"|"hsl"|"hsla"|"hwb"|"lab"|"lch"|"oklab"|"oklch"|"color"|"alpha"|
            "scale"|"scalex"|"scaley"|"scalez"|"scale3d"|"matrix"|"matrix3d"|
            "brightness"|"contrast"|"grayscale"|"invert"|"opacity"|"saturate"|"sepia"),
        None=>matches!(property,"opacity"|"fill-opacity"|"zoom"|"font-weight"|"font-size-adjust"|
            "order"|"z-index"|"column-count"|"orphans"|"widows"|"flex-grow"|"flex-shrink"|"scale"),
    }
}
pub(super) struct CustomResolver<'a> {
    pub(super) local:&'a BTreeMap<String,ScopedTokens>,
    pub(super) inherited:&'a BTreeMap<String,CustomValue>,
    pub(super) cycles:&'a BTreeSet<String>,
    pub(super) memo:BTreeMap<String,CustomValue>,
    pub(super) resolving:BTreeSet<String>,
    pub(super) tree:Option<&'a TreeContext<'a>>,
}
impl CustomResolver<'_> {
    pub(super) fn resolve_name(&mut self,name:&str)->CustomValue {
        if let Some(value)=self.memo.get(name) {return value.clone();}
        if self.cycles.contains(name)||!self.resolving.insert(name.into()) {return CustomValue::Invalid;}
        let value=match self.local.get(name).cloned() {
            Some(tokens)=>self.substitute_scoped(&tokens).map(CustomValue::Valid).unwrap_or(CustomValue::Invalid),
            None=>self.inherited.get(name).cloned().unwrap_or(CustomValue::Invalid),
        };
        self.resolving.remove(name);self.memo.insert(name.into(),value.clone());value
    }
    pub(super) fn substitute(&mut self,tokens:&[Token])->Option<Vec<Token>> {
        Some(self.substitute_scoped(&ScopedTokens::new(tokens.to_vec(),None))?.tokens)
    }
    pub(super) fn substitute_property(&mut self,name:&str,tokens:&[Token])->Option<Vec<Token>> {
        let scope=self.tree.and_then(|tree|tree.scope(name));
        let tokens=self.substitute_scoped(&ScopedTokens::new(tokens.to_vec(),scope))?;
        match self.tree {Some(tree)=>tree.resolve(name,tokens),None=>Some(tokens.tokens)}
    }
    fn substitute_scoped(&mut self,tokens:&ScopedTokens)->Option<ScopedTokens> {
        let mut output=ScopedTokens::new(Vec::new(),None);let mut index=0;
        while index<tokens.len() {
            if matches!(tokens.get(index),Some(Token::Function(name)) if name.eq_ignore_ascii_case("var")) {
                let call=parse_var_call(tokens,index)?;
                let replacement=match self.resolve_name(call.name) {
                    CustomValue::Valid(tokens)=>tokens,
                    CustomValue::Invalid=>{
                        let fallback=call.fallback?;
                        // The fallback borrows this exact input interval, so
                        // retain its per-token defining roots through recursion.
                        let start=call.fallback_start?;
                        self.substitute_scoped(&tokens.slice(start,start+fallback.len()))?
                    }
                };
                output.extend(replacement);index=call.end;
            } else {output.tokens.push(tokens[index].clone());output.scopes.push(tokens.scopes[index]);index+=1;}
        }
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compute,parse_stylesheet};
    use crate::selectors::PseudoElement;
    #[test]
    fn actual_dom_order_counts_only_elements_and_precedes_inheritance() {
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
        dom.create_element(html,"head");dom.create_text(html,"gap");
        let body=dom.create_element(html,"body");let first=dom.create_element(body,"i");let second=dom.create_element(body,"b");
        let css=parse_stylesheet("body{font-size:calc(10px * sibling-index());font-weight:calc(100 * sibling-count());background-image:linear-gradient(red calc(20px * sibling-index()),blue);--step:sibling-index()}i{font-size:inherit;width:calc(10px * var(--step))}b{font-size:inherit;width:calc(10px * var(--step));height:calc(10px * sibling-count())}body::before{content:'x';width:calc(10px * sibling-index());background-image:linear-gradient(red calc(20px * sibling-index()),blue)}");
        let result=compute(&dom,&[css]);let style=result.get(body).unwrap();
        assert_eq!(style.get("font-size"),Some("20px"));assert_eq!(style.get("font-weight"),Some("200"));
        assert!(style.get("background-image").unwrap().contains("40px"));
        for child in [first,second] {assert_eq!(result.get(child).unwrap().get("font-size"),Some("20px"));}
        // Unregistered custom tokens preserve source scope, but are evaluated
        // for the consuming element rather than eagerly for their parent.
        assert_eq!(result.get(first).unwrap().get("width"),Some("calc(10px * 1)"));
        assert_eq!(result.get(second).unwrap().get("width"),Some("calc(10px * 2)"));
        assert_eq!(result.get(second).unwrap().get("height"),Some("calc(10px * 2)"));
        let pseudo=result.get_pseudo(body,PseudoElement::Before).unwrap();
        assert_eq!(pseudo.get("width"),Some("calc(10px * 2)"));
        assert!(pseudo.get("background-image").unwrap().contains("40px"));
        assert_eq!(style.cssom_property_value("--step").as_deref(),Some("sibling-index()"));
    }
    #[test]
    fn every_nested_function_and_variable_fallback_keeps_its_actual_source() {
        let mut dom=Dom::new();let root=dom.create_element(NodeIdx(0),"section");
        dom.create_element(root,"i");let target=dom.create_element(root,"b");
        let mut sheet=parse_stylesheet("section{--index:sibling-index();--nested:var(--index)}b{width:calc(3px * var(--nested));height:calc(4px * var(--absent,sibling-count()));opacity:calc(sibling-index() / sibling-count());transform:scale(sibling-index(),sibling-count());background-image:linear-gradient(red min(calc(10px * sibling-index()),30px),blue)}");
        sheet.base_url=Some("https://example.test/theme.css".into());sheet.resource_context=Some(8);
        let result=compute(&dom,&[sheet]);let style=result.get(target).unwrap();
        assert_eq!(style.get("width"),Some("calc(3px * 2)"));assert_eq!(style.get("height"),Some("calc(4px * 2)"));
        assert_eq!(style.get("opacity"),Some("1"));
        assert_eq!(style.get("transform"),Some("scale(2, 2)"));
        assert!(style.get("background-image").unwrap().contains("20px"));
        assert_eq!(style.url_base("background-image"),Some("https://example.test/theme.css"));assert_eq!(style.url_context("background-image"),Some(8));
    }
    struct Metrics;
    impl crate::font_metric_context::FontMetricContext for Metrics {
        fn aspect_ratio(&self,_:NodeIdx,_:Option<PseudoElement>,_:&ComputedStyle,_:crate::font_size_adjust::FontSizeAdjustMetric)->Option<f64> {None}
    }
    fn shadow(dom:&mut Dom,host:NodeIdx,mode:amazing_dom::shadow::ShadowMode)->NodeIdx {
        dom.attach_shadow(host,amazing_dom::shadow::ShadowOptions{mode,slot_assignment:amazing_dom::shadow::SlotAssignment::Named,
            delegates_focus:false,clonable:false,serializable:false}).unwrap()
    }
    #[test]
    fn open_closed_nested_host_slot_and_inherited_reference_scopes() {
        for mode in [amazing_dom::shadow::ShadowMode::Open,amazing_dom::shadow::ShadowMode::Closed] {
            let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
            dom.create_element(html,"aside");let host=dom.create_element(html,"section");
            dom.create_element(host,"i");let assigned=dom.create_element(host,"b");
            let root=shadow(&mut dom,host,mode);let slot=dom.create_element(root,"slot");
            let inner=dom.create_element(root,"div");let nested=dom.create_element(root,"article");
            let deep_root=shadow(&mut dom,nested,mode);let deep=dom.create_element(deep_root,"span");
            let document=[parse_stylesheet("section{--outer:sibling-count();font-size:calc(10px * sibling-index())}")];
            let scopes=[(root,vec![parse_stylesheet(":host{width:calc(10px * sibling-index())}::slotted(b){width:calc(10px * sibling-index())}slot{--slot-index:sibling-index()}div{width:calc(10px * sibling-index());height:calc(10px * var(--outer));--inside:sibling-index()}article{--inside:sibling-index()}")]),
                (deep_root,vec![parse_stylesheet("span{width:calc(10px * sibling-index());height:calc(10px * var(--inside));min-height:calc(10px * var(--missing,sibling-count()))}")])];
            let result=crate::compute_document_with_shadow_font_metrics(&dom,NodeIdx(0),&document,&scopes,&MediaEnvironment::default(),None,&Metrics);
            assert_eq!(result.get(host).unwrap().get("width"),Some("calc(10px * 2)"));
            assert_eq!(result.get(assigned).unwrap().get("width"),Some("calc(10px * 2)"));
            assert_eq!(result.get(inner).unwrap().get("width"),Some("calc(10px * 2)"));
            // The automatic name is in the descendant tree, unavailable to
            // an outer defining reference inherited through a custom value.
            assert_eq!(result.get(inner).unwrap().get("height"),Some("calc(10px * 0)"));
            assert_eq!(result.get(deep).unwrap().get("width"),Some("calc(10px * 1)"));
            assert_eq!(result.get(deep).unwrap().get("height"),Some("calc(10px * 0)"));
            assert_eq!(result.get(deep).unwrap().get("min-height"),Some("calc(10px * 1)"));
            assert!(result.get(slot).is_some());
        }
    }
    #[test]
    fn native_document_identity_and_invalid_counting_arguments_are_atomic() {
        let mut dom=Dom::new();dom.create_element(NodeIdx(0),"html");
        let document=dom.append_child(NodeIdx(0),amazing_dom::NodeKind::Document);
        let html=dom.create_element(document,"html");dom.create_element(html,"head");let body=dom.create_element(html,"body");
        let sheets=[parse_stylesheet("body{font-size:calc(10px * sibling-index());height:calc(10px * sibling-count(2));background-image:linear-gradient(red calc(20px * sibling-index()),blue)}")];
        let result=crate::compute_document_with_font_metrics(&dom,document,&sheets,&MediaEnvironment::default(),None,&Metrics);
        assert_eq!(result.get(body).unwrap().get("font-size"),Some("20px"));
        assert_eq!(result.get(body).unwrap().get("height"),Some("auto"));
        assert!(result.get(body).unwrap().get("background-image").unwrap().contains("40px"));
        for css in ["calc(2px * sibling-index(1))","calc(2px * sibling-count(foo))"] {assert!(!crate::math::length_percentage(css,true));}
        for css in ["calc(2px * sibling-index())","calc(2px * sibling-count( ))"] {assert!(crate::math::length_percentage(css,true));}
    }
    #[test]
    fn numeric_functions_never_acquire_literal_zero_dimension_exceptions() {
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
        let host=dom.create_element(html,"section");let root=shadow(&mut dom,host,amazing_dom::shadow::ShadowMode::Open);
        let inner=dom.create_element(root,"div");
        let document=[parse_stylesheet("section{--outer:sibling-index()}")];
        let sheets=[(root,vec![parse_stylesheet("div{width:var(--outer);height:calc(var(--outer));margin-left:var(--outer);background-position-x:var(--outer);transform:translateX(var(--outer));opacity:var(--outer);z-index:var(--outer)}")])];
        let styles=crate::compute_document_with_shadow_font_metrics(&dom,NodeIdx(0),&document,&sheets,&MediaEnvironment::default(),None,&Metrics);
        let style=styles.get(inner).unwrap();
        for name in ["width","height"] {assert_eq!(style.get(name),Some("auto"),"{name}");}
        assert_eq!(style.get("margin-left"),Some("0"));
        assert_eq!(style.get("background-position-x"),Some("0%"));
        assert_eq!(style.get("transform"),Some("none"));
        assert_eq!(style.get("opacity"),Some("0"));assert_eq!(style.get("z-index"),Some("0"));
        let styles=compute(&dom,&[parse_stylesheet("section{width:sibling-index();opacity:sibling-index();transform:scale(sibling-index())}")]);
        let style=styles.get(host).unwrap();assert_eq!(style.get("width"),Some("auto"));
        assert_eq!(style.get("opacity"),Some("1"));assert_eq!(style.get("transform"),Some("scale(1)"));
    }
}
