//! CSS 2.1 §17.2.1 anonymous table objects. Repair parents before recursing:
//! repairing each DOM element independently would wrap every orphan cell in
//! its own table and lose consecutive-sibling grouping.
use crate::{Display, Flow, LayoutBox};
use amazing_css::ComputedStyle;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Role {
    #[default]
    Other,
    Table,
    RowGroup,
    Header,
    Footer,
    Row,
    Cell,
    Column,
    ColumnGroup,
    Caption,
}
impl Role {
    pub(crate) fn of(style: &ComputedStyle) -> Self {
        match style
            .get("display")
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "table" | "inline-table" => Self::Table,
            "table-row-group" => Self::RowGroup,
            "table-header-group" => Self::Header,
            "table-footer-group" => Self::Footer,
            "table-row" => Self::Row,
            "table-cell" => Self::Cell,
            "table-column" => Self::Column,
            "table-column-group" => Self::ColumnGroup,
            "table-caption" => Self::Caption,
            _ => Self::Other,
        }
    }
    pub(crate) fn group(self) -> bool {
        matches!(self, Self::RowGroup | Self::Header | Self::Footer)
    }
    fn proper(self) -> bool {
        self.group()
            || matches!(
                self,
                Self::Row | Self::Column | Self::ColumnGroup | Self::Caption
            )
    }
    fn display(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Row => "table-row",
            Self::Cell => "table-cell",
            _ => "block",
        }
    }
}

pub(crate) fn role(b: &LayoutBox) -> Role {
    Role::of(&b.style)
}

fn anonymous(parent: &LayoutBox, role: Role, children: Vec<LayoutBox>) -> LayoutBox {
    let mut style = ComputedStyle::default();
    // Anonymous boxes inherit computed inherited values; they do not inherit
    // widths, margins, padding, backgrounds, positions or DOM span attributes.
    for name in [
        "color",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "font-stretch",
        "line-height",
        "white-space",
        "direction",
        "text-align",
        "writing-mode",
        "border-collapse",
        "border-spacing",
        "caption-side",
        "empty-cells",
    ] {
        if let Some(value) = parent.style.get(name) {
            style.set(name, value);
        }
    }
    style.set("display", role.display());
    let mut b = LayoutBox::new(
        None,
        Display::Block,
        style,
        parent.font_size,
        parent.line_height,
    );
    b.flow = Flow::Blocks(children);
    b
}

fn wrap_runs(
    parent: &LayoutBox,
    children: Vec<LayoutBox>,
    wrapper: Role,
    select: impl Fn(Role) -> bool,
) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    let mut run = Vec::new();
    for child in children {
        if select(role(&child)) {
            run.push(child);
        } else {
            if !run.is_empty() {
                out.push(anonymous(parent, wrapper, std::mem::take(&mut run)));
            }
            out.push(child);
        }
    }
    if !run.is_empty() {
        out.push(anonymous(parent, wrapper, run));
    }
    out
}

pub(crate) fn normalize(b: &mut LayoutBox) {
    for child in &mut b.positioned {
        normalize(child);
    }
    let r = role(b);
    if r == Role::Column {
        b.flow = Flow::Empty;
        b.positioned.clear();
        return;
    }
    let original = std::mem::replace(&mut b.flow, Flow::Empty);
    let mut children = match original {
        Flow::Blocks(children) => children,
        Flow::Inlines(items) if r == Role::Table || r.group() || r == Role::Row => {
            let mut inline = anonymous(b, Role::Other, Vec::new());
            inline.flow = Flow::Inlines(items);
            vec![inline]
        }
        Flow::Flex(mut children) => {
            for child in &mut children {
                normalize(child);
            }
            b.flow = Flow::Flex(children);
            return;
        }
        Flow::Grid(mut children) => {
            for child in &mut children {
                normalize(child);
            }
            b.flow = Flow::Grid(children);
            return;
        }
        other => {
            b.flow = other;
            if r == Role::Table {
                wrap_table(b);
            }
            return;
        }
    };
    if r == Role::ColumnGroup {
        children.retain(|c| role(c) == Role::Column);
    } else {
        // Missing child wrappers precede missing-parent repair.
        if r == Role::Table {
            children = wrap_runs(b, children, Role::Row, |c| !c.proper());
        } else if r.group() {
            children = wrap_runs(b, children, Role::Row, |c| c != Role::Row);
        } else if r == Role::Row {
            children = wrap_runs(b, children, Role::Cell, |c| c != Role::Cell);
        }
        if r != Role::Row {
            children = wrap_runs(b, children, Role::Row, |c| c == Role::Cell);
        }
        if r != Role::Table {
            // A row remains legal in a row group, and a column in a colgroup.
            children = wrap_runs(b, children, Role::Table, |c| {
                c.proper() && !(r.group() && c == Role::Row)
            });
        }
    }
    for child in &mut children {
        normalize(child);
    }
    b.flow = Flow::Blocks(children);
    if r == Role::Table {
        wrap_table(b);
    }
}

/// CSS 2.1 §17.4: margins/position/float belong to the wrapper; backgrounds,
/// borders and sizing belong to the grid. Captions must not enlarge the grid's
/// painted background or border rectangle.
fn wrap_table(b: &mut LayoutBox) {
    let mut wrapper = anonymous(b, Role::Other, Vec::new());
    wrapper.table_wrapper = true;
    // The principal wrapper retains source identity for paint ordering; the
    // inner grid uses the same element identity for table style and hit data.
    wrapper.node = b.node;
    wrapper.style.set("display", "flow-root");
    for name in [
        "position",
        "float",
        "clear",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "top",
        "right",
        "bottom",
        "left",
        "z-index",
    ] {
        if let Some(v) = b.style.get(name) {
            wrapper.style.set(name, v);
        }
        let initial = match name {
            "position" => "static",
            "float" | "clear" => "none",
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => "0",
            _ => "auto",
        };
        b.style.set(name, initial);
    }
    let mut captions = Vec::new();
    if let Flow::Blocks(c) = &mut b.flow {
        let old = std::mem::take(c);
        for child in old {
            if role(&child) == Role::Caption {
                captions.push(child);
            } else {
                c.push(child);
            }
        }
    }
    wrapper.positioned = std::mem::take(&mut b.positioned);
    let table = std::mem::replace(b, wrapper);
    let mut children = vec![table];
    children.extend(captions);
    b.flow = Flow::Blocks(children);
}
