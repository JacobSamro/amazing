use amazing_dom::NodeIdx;
use crate::{parse, parse_fragment, IncrementalParser};

#[test]
fn implied_body_is_replaced_only_while_frameset_is_allowed() {
    let frameset = parse("<p> <frameset><frame>").dump(NodeIdx(0));
    assert!(frameset.contains("<frameset>\n      <frame>"), "{frameset}");
    assert!(!frameset.contains("<body>"), "{frameset}");

    for source in ["<body><frameset><frame>", "<p>x<frameset><frame>",
        "<svg>x</svg><frameset><frame>"] {
        let body = parse(source).dump(NodeIdx(0));
        assert!(body.contains("<body>"), "{source}: {body}");
        assert!(!body.contains("<frameset>"), "{source}: {body}");
    }
}

#[test]
fn frameset_replacement_survives_chunk_splits_and_stays_out_of_fragments() {
    let source = "<div><svg> </svg><frameset><frame></frameset></html>  ";
    let expected = parse(source).dump(NodeIdx(0));
    let mut parser = IncrementalParser::new();
    for chunk in ["<di", "v><sv", "g> </svg><frame", "set><frame></frame", "set></html> ", " "] {
        parser.push_chunk(chunk);
    }
    assert_eq!(parser.finish().dump(NodeIdx(0)), expected);
    assert!(expected.contains("<frameset>"), "{expected}");

    let fragment = parse_fragment("div", "<frameset><frame><p>kept</p>").unwrap();
    let dump = fragment.dump(NodeIdx(0));
    assert!(!dump.contains("<frameset>"), "{dump}");
    assert!(dump.contains("<p>"), "{dump}");
}
