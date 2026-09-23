//! Canvas WebIDL member inventory and retained bitmap state.
//!
//! The runtime installs these members directly on the existing HTMLCanvasElement
//! prototype. This avoids changing the shared DOM interface tables while the
//! canvas lane owns its JS surface.
use amazing_canvas::Canvas2D;
use amazing_dom::NodeIdx;

/// HTML non-negative integer parsing used by canvas dimension reflection.
pub fn parse_dimension(input: &str, default: u32) -> u32 {
    let input = input.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
    let input = input.strip_prefix('+').unwrap_or(input);
    let digits = input.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return default;
    }
    input[..digits].parse().unwrap_or(default)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method(u8),
    Getter,
    Setter,
    Constructor(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: &'static str,
    pub kind: MemberKind,
}

const fn method(name: &'static str, length: u8) -> Member {
    Member {
        name,
        kind: MemberKind::Method(length),
    }
}
const fn getter(name: &'static str) -> Member {
    Member {
        name,
        kind: MemberKind::Getter,
    }
}
const fn setter(name: &'static str) -> Member {
    Member {
        name,
        kind: MemberKind::Setter,
    }
}

pub const ELEMENT: &[Member] = &[
    getter("width"),
    setter("width"),
    getter("height"),
    setter("height"),
    method("getContext", 1),
    method("toDataURL", 0),
    method("toBlob", 1),
];

pub const CONTEXT: &[Member] = &[
    getter("canvas"),
    setter("canvas"),
    getter("fillStyle"),
    setter("fillStyle"),
    getter("strokeStyle"),
    setter("strokeStyle"),
    getter("globalAlpha"),
    setter("globalAlpha"),
    getter("globalCompositeOperation"),
    setter("globalCompositeOperation"),
    getter("lineWidth"),
    setter("lineWidth"),
    getter("lineCap"),
    setter("lineCap"),
    getter("lineJoin"),
    setter("lineJoin"),
    getter("miterLimit"),
    setter("miterLimit"),
    getter("lineDashOffset"),
    setter("lineDashOffset"),
    getter("shadowColor"),
    setter("shadowColor"),
    getter("shadowBlur"),
    setter("shadowBlur"),
    getter("shadowOffsetX"),
    setter("shadowOffsetX"),
    getter("shadowOffsetY"),
    setter("shadowOffsetY"),
    getter("font"),
    setter("font"),
    getter("textAlign"),
    setter("textAlign"),
    getter("textBaseline"),
    setter("textBaseline"),
    getter("direction"),
    setter("direction"),
    getter("imageSmoothingEnabled"),
    setter("imageSmoothingEnabled"),
    getter("imageSmoothingQuality"),
    setter("imageSmoothingQuality"),
    getter("filter"),
    setter("filter"),
    method("save", 0),
    method("restore", 0),
    method("reset", 0),
    method("getContextAttributes", 0),
    method("scale", 2),
    method("rotate", 1),
    method("translate", 2),
    method("transform", 6),
    method("setTransform", 0),
    method("resetTransform", 0),
    method("getTransform", 0),
    method("createLinearGradient", 4),
    method("createRadialGradient", 6),
    method("createConicGradient", 3),
    method("createPattern", 2),
    method("clearRect", 4),
    method("fillRect", 4),
    method("strokeRect", 4),
    method("beginPath", 0),
    method("closePath", 0),
    method("moveTo", 2),
    method("lineTo", 2),
    method("quadraticCurveTo", 4),
    method("bezierCurveTo", 6),
    method("arcTo", 5),
    method("rect", 4),
    method("roundRect", 5),
    method("arc", 5),
    method("ellipse", 7),
    method("fill", 0),
    method("stroke", 0),
    method("clip", 0),
    method("isPointInPath", 2),
    method("isPointInStroke", 2),
    method("setLineDash", 1),
    method("getLineDash", 0),
    method("fillText", 3),
    method("strokeText", 3),
    method("measureText", 1),
    method("drawImage", 3),
    method("createImageData", 2),
    method("getImageData", 4),
    method("putImageData", 3),
];

pub const PATH: &[Member] = &[
    method("addPath", 1),
    method("closePath", 0),
    method("moveTo", 2),
    method("lineTo", 2),
    method("quadraticCurveTo", 4),
    method("bezierCurveTo", 6),
    method("arcTo", 5),
    method("rect", 4),
    method("roundRect", 5),
    method("arc", 5),
    method("ellipse", 7),
];

pub const GRADIENT: &[Member] = &[method("addColorStop", 2)];
pub const IMAGE_DATA: &[Member] = &[
    getter("width"),
    getter("height"),
    getter("data"),
    getter("colorSpace"),
];
pub const TEXT_METRICS: &[Member] = &[
    getter("width"),
    getter("actualBoundingBoxLeft"),
    getter("actualBoundingBoxRight"),
    getter("fontBoundingBoxAscent"),
    getter("fontBoundingBoxDescent"),
    getter("actualBoundingBoxAscent"),
    getter("actualBoundingBoxDescent"),
    getter("emHeightAscent"),
    getter("emHeightDescent"),
    getter("hangingBaseline"),
    getter("alphabeticBaseline"),
    getter("ideographicBaseline"),
];

/// A canvas retains one rendering context for its whole lifetime. Resizing
/// resets its bitmap, state stack and origin-clean flag, even if dimensions
/// are set to their previous values.
pub struct CanvasSlot {
    pub node: NodeIdx,
    pub context: Canvas2D,
    pub origin_clean: bool,
}

impl CanvasSlot {
    pub fn new(node: NodeIdx, width: u32, height: u32) -> Self {
        Self {
            node,
            context: Canvas2D::new(width, height),
            origin_clean: true,
        }
    }
    pub fn reset(&mut self, width: u32, height: u32) {
        self.context = Canvas2D::new(width, height);
        self.origin_clean = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_resets_pixels_and_origin_clean() {
        let mut slot = CanvasSlot::new(NodeIdx(1), 2, 2);
        slot.context.set_fill_color("red");
        slot.context.fill_rect(0., 0., 2., 2.);
        slot.origin_clean = false;
        slot.reset(2, 2);
        assert!(slot.origin_clean);
        assert_eq!(slot.context.surface.pixels, vec![0; 16]);
    }
    #[test]
    fn html_dimension_parser_accepts_prefix_digits() {
        assert_eq!(parse_dimension("  +12px", 300), 12);
        assert_eq!(parse_dimension("-1", 300), 300);
        assert_eq!(parse_dimension("0x20", 300), 0);
        assert_eq!(parse_dimension("", 300), 300);
    }
}
