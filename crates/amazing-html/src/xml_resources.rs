//! Explicit application-owned external XML input. No implicit IO is installed.
//! A source descriptor is parsing provenance, never browser fetch authority.
use crate::xml::{XmlError, XmlErrorKind};
use std::collections::BTreeSet;

/// Actual identity/base supplied by the application that accepted the bytes.
/// Resolvers must use equal identities for aliases of the same native resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlEntitySource {
    identity: String,
    base: String,
}
impl XmlEntitySource {
    pub fn new(identity: String, base: String) -> Result<Self, XmlError> {
        if identity.is_empty() || base.is_empty() {
            return Err(error(
                XmlErrorKind::Unsupported,
                "external XML requires explicit source identity and base",
            ));
        }
        Ok(Self { identity, base })
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn base(&self) -> &str {
        &self.base
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlExternalKind {
    Subset,
    ParameterEntity,
    GeneralEntity,
}
/// Declaration base is retained from the entity containing the declaration.
/// SYSTEM/PUBLIC text is a resolution input, not authorization to perform IO.
pub struct XmlExternalRequest<'a> {
    pub declaration_source: &'a XmlEntitySource,
    pub system_id: &'a str,
    pub public_id: Option<&'a str>,
    pub kind: XmlExternalKind,
}
/// Actual byte authority supplied by the resource producer. No MIME response
/// is synthesized when the application reads a local parsed entity.
#[derive(Clone, Debug)]
pub enum XmlExternalEncoding {
    Entity,
    Mime { charset: Option<String> },
}
/// A successful application read with its actual entity/MIME byte authority.
pub struct XmlExternalResource {
    pub source: XmlEntitySource,
    pub bytes: Vec<u8>,
    pub encoding_authority: XmlExternalEncoding,
}
/// The application owns resource access policy. None means the entity was not
/// read; failure stays a typed error. The parser never substitutes a response.
pub trait XmlExternalResolver {
    fn resolve(
        &mut self,
        request: XmlExternalRequest<'_>,
    ) -> Result<Option<XmlExternalResource>, XmlError>;
}
fn error(kind: XmlErrorKind, message: &str) -> XmlError {
    XmlError {
        offset: 0,
        kind,
        message: message.into(),
    }
}

/// One parse's actual external resource accounting, separate from declaration
/// expansion accounting. No global cache or default resolver is installed.
pub(super) struct XmlResourceSession<'a> {
    resolver: &'a mut dyn XmlExternalResolver,
    active: BTreeSet<String>,
    stack: Vec<String>,
    reads: usize,
    bytes: usize,
}
impl<'a> XmlResourceSession<'a> {
    pub fn new(resolver: &'a mut dyn XmlExternalResolver) -> Self {
        Self {
            resolver,
            active: BTreeSet::new(),
            stack: Vec::new(),
            reads: 0,
            bytes: 0,
        }
    }
    pub fn read(
        &mut self,
        request: XmlExternalRequest<'_>,
    ) -> Result<Option<XmlExternalResource>, XmlError> {
        if self.stack.len() >= 32 {
            return Err(error(
                XmlErrorKind::Limit,
                "external XML resource nesting exceeds 32",
            ));
        }
        self.reads = self
            .reads
            .checked_add(1)
            .ok_or_else(|| error(XmlErrorKind::Limit, "external XML read budget overflow"))?;
        if self.reads > 16_384 {
            return Err(error(
                XmlErrorKind::Limit,
                "external XML read budget exceeded",
            ));
        }
        let Some(resource) = self.resolver.resolve(request)? else {
            return Ok(None);
        };
        if self.active.contains(resource.source.identity()) {
            return Err(error(
                XmlErrorKind::Malformed,
                "recursive external XML resource",
            ));
        }
        self.bytes = self
            .bytes
            .checked_add(resource.bytes.len())
            .ok_or_else(|| error(XmlErrorKind::Limit, "external XML byte budget overflow"))?;
        if self.bytes > 16 * 1024 * 1024 {
            return Err(error(
                XmlErrorKind::Limit,
                "external XML input exceeds 16 MiB",
            ));
        }
        Ok(Some(resource))
    }
    /// Called only while the accepted resource is actually being parsed.
    pub fn enter(&mut self, source: &XmlEntitySource) -> Result<(), XmlError> {
        if self.stack.len() >= 32 {
            return Err(error(
                XmlErrorKind::Limit,
                "external XML resource nesting exceeds 32",
            ));
        }
        if !self.active.insert(source.identity.clone()) {
            return Err(error(
                XmlErrorKind::Malformed,
                "recursive external XML resource",
            ));
        }
        self.stack.push(source.identity.clone());
        Ok(())
    }
    pub fn leave(&mut self) {
        if let Some(identity) = self.stack.pop() {
            self.active.remove(&identity);
        }
    }
}

/// Explicit local-file application policy. Constructing it declares the only
/// canonical directory from which this resolver is allowed to accept bytes.
/// Browser network consumers use their own resolver and native client policy.
pub struct XmlFileResolver {
    root: std::path::PathBuf,
}
impl XmlFileResolver {
    pub fn new(root: &std::path::Path) -> Result<Self, XmlError> {
        let root = root.canonicalize().map_err(|_| {
            error(
                XmlErrorKind::Unsupported,
                "XML resource root is unavailable",
            )
        })?;
        if !root.is_dir() {
            return Err(error(
                XmlErrorKind::Unsupported,
                "XML resource root is not a directory",
            ));
        }
        Ok(Self { root })
    }
    pub fn read_document(&self, path: &std::path::Path) -> Result<XmlExternalResource, XmlError> {
        self.read_file(path)
    }
    fn read_file(&self, path: &std::path::Path) -> Result<XmlExternalResource, XmlError> {
        use std::io::Read;
        let actual = path
            .canonicalize()
            .map_err(|_| error(XmlErrorKind::Unsupported, "XML resource was not read"))?;
        if !actual.starts_with(&self.root) || !actual.is_file() {
            return Err(error(
                XmlErrorKind::Unsupported,
                "XML resource is outside the declared file policy",
            ));
        }
        let mut file = std::fs::File::open(&actual)
            .map_err(|_| error(XmlErrorKind::Unsupported, "XML resource was not read"))?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error(XmlErrorKind::Unsupported, "XML resource read failed"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(error(
                XmlErrorKind::Limit,
                "external XML input exceeds 16 MiB",
            ));
        }
        let name = actual.to_str().ok_or_else(|| {
            error(
                XmlErrorKind::Unsupported,
                "XML resource identity is not UTF-8",
            )
        })?;
        Ok(XmlExternalResource {
            source: XmlEntitySource::new(name.into(), name.into())?,
            bytes,
            encoding_authority: XmlExternalEncoding::Entity,
        })
    }
}
impl XmlExternalResolver for XmlFileResolver {
    fn resolve(
        &mut self,
        request: XmlExternalRequest<'_>,
    ) -> Result<Option<XmlExternalResource>, XmlError> {
        if request.system_id.contains('#') {
            return Err(error(
                XmlErrorKind::Malformed,
                "fragment identifier in XML system identifier",
            ));
        }
        let base = std::path::Path::new(request.declaration_source.base());
        let actual_base = base.canonicalize().map_err(|_| {
            error(
                XmlErrorKind::Unsupported,
                "XML declaration base is unavailable",
            )
        })?;
        if !actual_base.starts_with(&self.root)
            || !actual_base.is_file()
            || actual_base.to_str() != Some(request.declaration_source.identity())
        {
            return Err(error(
                XmlErrorKind::Unsupported,
                "XML declaration has no accepted local-file source",
            ));
        }
        let raw = request
            .system_id
            .split_once('?')
            .map_or(request.system_id, |(path, _)| path);
        if raw.is_empty() {
            return self.read_file(&actual_base).map(Some);
        }
        let path = if raw
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
        {
            let rest = &raw[5..];
            if let Some(authority_path) = rest.strip_prefix("//") {
                if authority_path.starts_with('/') {
                    authority_path
                } else {
                    let Some((authority, path)) = authority_path.split_once('/') else {
                        return Ok(None);
                    };
                    if !authority.eq_ignore_ascii_case("localhost") {
                        return Ok(None);
                    }
                    return self.resolve(XmlExternalRequest {
                        system_id: &format!("file:/{path}"),
                        ..request
                    });
                }
            } else {
                rest
            }
        } else {
            if raw.split('/').next().is_some_and(|part| part.contains(':')) {
                return Ok(None);
            }
            raw
        };
        let mut decoded = Vec::with_capacity(path.len());
        let mut input = path.as_bytes().iter().copied();
        while let Some(byte) = input.next() {
            if byte == b'%' {
                let digit = |b: u8| (b as char).to_digit(16);
                let hi = input.next().and_then(digit);
                let lo = input.next().and_then(digit);
                let (Some(hi), Some(lo)) = (hi, lo) else {
                    return Err(error(
                        XmlErrorKind::Malformed,
                        "invalid percent escape in XML system identifier",
                    ));
                };
                decoded.push((hi * 16 + lo) as u8);
            } else {
                decoded.push(byte);
            }
        }
        let decoded = String::from_utf8(decoded)
            .map_err(|_| error(XmlErrorKind::Unsupported, "XML file reference is not UTF-8"))?;
        let parent = actual_base.parent().ok_or_else(|| {
            error(
                XmlErrorKind::Unsupported,
                "XML declaration has no resource base",
            )
        })?;
        let target = parent.join(decoded);
        match target.canonicalize() {
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                Ok(None)
            }
            Err(_) => Err(error(
                XmlErrorKind::Unsupported,
                "XML resource path resolution failed",
            )),
            Ok(actual) if !actual.starts_with(&self.root) || !actual.is_file() => Ok(None),
            Ok(actual) => self.read_file(&actual).map(Some),
        }
    }
}
