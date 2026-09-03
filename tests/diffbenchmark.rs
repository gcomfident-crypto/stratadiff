use stratadiff::diffbenchmark::{
    ComparableNode, GodReport, JdtOracleMapping, JdtOracleNode, OffsetRange, OracleMapping,
    OracleNode, SharedNodeRole, comparable_tree_sitter_java_nodes, jdt_node_role,
    normalize_oracle_mapping, parse_god_info, parse_god_report, parse_oracle_mapping,
    resolve_jdt_node, tree_sitter_java_node_role, utf16_offset_to_byte_offset,
};

#[test]
fn parses_the_complete_god_report_shape() {
    let report = parse_god_report(
        br#"{
          "intraFileMappings": {
            "matchedElements": [
              {"left":"A","right":"B","info":"TypeDeclaration[0-1]:TypeDeclaration[2-3]"}
            ],
            "mappings": []
          },
          "interFileMappings": {
            "Moved to File: Other.java": {
              "matchedElements": [],
              "mappings": [
                {"left":"x","right":"y","info":"SimpleName[4-5]:SimpleName[6-7]"}
              ]
            }
          }
        }"#,
    )
    .unwrap();

    assert_eq!(report.intra_file_mappings.matched_elements.len(), 1);
    assert_eq!(report.inter_file_mappings.len(), 1);
    assert_eq!(
        report.inter_file_mappings["Moved to File: Other.java"].mappings[0].left,
        "x"
    );
}

#[test]
fn god_report_parser_rejects_unknown_fields_and_invalid_json() {
    let unknown = br#"{
      "intraFileMappings":{"matchedElements":[],"mappings":[],"extra":true},
      "interFileMappings":{}
    }"#;
    assert!(parse_god_report(unknown).is_err());
    assert!(parse_god_report(br#"{"intraFileMappings": "#).is_err());

    let report = GodReport {
        intra_file_mappings: stratadiff::diffbenchmark::GodMappingGroup {
            matched_elements: Vec::new(),
            mappings: Vec::new(),
        },
        inter_file_mappings: std::collections::BTreeMap::new(),
    };
    assert_eq!(
        serde_json::from_value::<GodReport>(serde_json::to_value(&report).unwrap()).unwrap(),
        report
    );
}

#[test]
fn jdt_and_tree_sitter_types_share_explicit_roles() {
    let cases = [
        (
            "MethodDeclaration",
            "method_declaration",
            SharedNodeRole::MethodDeclaration,
        ),
        (
            "FieldDeclaration",
            "field_declaration",
            SharedNodeRole::FieldDeclaration,
        ),
        (
            "TypeDeclaration",
            "class_declaration",
            SharedNodeRole::TypeDeclaration,
        ),
        (
            "TYPE_DECLARATION_KIND",
            "class",
            SharedNodeRole::TypeDeclarationKind,
        ),
        (
            "EnumDeclaration",
            "enum_declaration",
            SharedNodeRole::EnumDeclaration,
        ),
        (
            "EnumConstantDeclaration",
            "enum_constant",
            SharedNodeRole::EnumConstantDeclaration,
        ),
        (
            "AnnotationTypeDeclaration",
            "annotation_type_declaration",
            SharedNodeRole::AnnotationTypeDeclaration,
        ),
        (
            "RecordDeclaration",
            "record_declaration",
            SharedNodeRole::RecordDeclaration,
        ),
        ("Block", "block", SharedNodeRole::Block),
        (
            "AssertStatement",
            "assert_statement",
            SharedNodeRole::AssertStatement,
        ),
        (
            "BreakStatement",
            "break_statement",
            SharedNodeRole::BreakStatement,
        ),
        (
            "ContinueStatement",
            "continue_statement",
            SharedNodeRole::ContinueStatement,
        ),
        ("DoStatement", "do_statement", SharedNodeRole::DoStatement),
        (
            "EnhancedForStatement",
            "enhanced_for_statement",
            SharedNodeRole::EnhancedForStatement,
        ),
        (
            "ConstructorInvocation",
            "explicit_constructor_invocation",
            SharedNodeRole::ExplicitConstructorInvocation,
        ),
        (
            "ExpressionStatement",
            "expression_statement",
            SharedNodeRole::ExpressionStatement,
        ),
        (
            "ForStatement",
            "for_statement",
            SharedNodeRole::ForStatement,
        ),
        ("IfStatement", "if_statement", SharedNodeRole::IfStatement),
        (
            "LabeledStatement",
            "labeled_statement",
            SharedNodeRole::LabeledStatement,
        ),
        (
            "VariableDeclarationStatement",
            "local_variable_declaration",
            SharedNodeRole::LocalVariableDeclaration,
        ),
        (
            "ReturnStatement",
            "return_statement",
            SharedNodeRole::ReturnStatement,
        ),
        (
            "SwitchStatement",
            "switch_expression",
            SharedNodeRole::SwitchConstruct,
        ),
        (
            "SynchronizedStatement",
            "synchronized_statement",
            SharedNodeRole::SynchronizedStatement,
        ),
        (
            "ThrowStatement",
            "throw_statement",
            SharedNodeRole::ThrowStatement,
        ),
        (
            "TryStatement",
            "try_statement",
            SharedNodeRole::TryStatement,
        ),
        (
            "WhileStatement",
            "while_statement",
            SharedNodeRole::WhileStatement,
        ),
        (
            "YieldStatement",
            "yield_statement",
            SharedNodeRole::YieldStatement,
        ),
        ("SimpleName", "identifier", SharedNodeRole::SimpleName),
        (
            "PrimitiveType",
            "integral_type",
            SharedNodeRole::PrimitiveType,
        ),
        ("SimpleType", "type_identifier", SharedNodeRole::SimpleType),
        ("ArrayType", "array_type", SharedNodeRole::ArrayType),
        (
            "ParameterizedType",
            "generic_type",
            SharedNodeRole::ParameterizedType,
        ),
        ("Modifier", "public", SharedNodeRole::Modifier),
        ("BooleanLiteral", "true", SharedNodeRole::BooleanLiteral),
        (
            "CharacterLiteral",
            "character_literal",
            SharedNodeRole::CharacterLiteral,
        ),
        (
            "NumberLiteral",
            "decimal_integer_literal",
            SharedNodeRole::NumberLiteral,
        ),
        (
            "StringLiteral",
            "string_literal",
            SharedNodeRole::StringLiteral,
        ),
        ("NullLiteral", "null_literal", SharedNodeRole::NullLiteral),
        ("TypeLiteral", "class_literal", SharedNodeRole::TypeLiteral),
    ];

    for (jdt, tree_sitter, expected) in cases {
        assert_eq!(jdt_node_role(jdt), Some(expected), "JDT type {jdt}");
        assert_eq!(
            tree_sitter_java_node_role(tree_sitter),
            Some(expected),
            "tree-sitter type {tree_sitter}"
        );
    }
}

#[test]
fn parser_specific_aliases_map_only_to_their_declared_roles() {
    let jdt_aliases = [
        (
            "SuperConstructorInvocation",
            SharedNodeRole::ExplicitConstructorInvocation,
        ),
        ("SwitchExpression", SharedNodeRole::SwitchConstruct),
        ("TextBlock", SharedNodeRole::StringLiteral),
    ];
    let tree_sitter_aliases = [
        ("constructor_declaration", SharedNodeRole::MethodDeclaration),
        (
            "compact_constructor_declaration",
            SharedNodeRole::MethodDeclaration,
        ),
        ("constant_declaration", SharedNodeRole::FieldDeclaration),
        ("interface_declaration", SharedNodeRole::TypeDeclaration),
        ("interface", SharedNodeRole::TypeDeclarationKind),
        ("constructor_body", SharedNodeRole::Block),
        ("try_with_resources_statement", SharedNodeRole::TryStatement),
        ("floating_point_type", SharedNodeRole::PrimitiveType),
        ("boolean_type", SharedNodeRole::PrimitiveType),
        ("void_type", SharedNodeRole::PrimitiveType),
        ("private", SharedNodeRole::Modifier),
        ("protected", SharedNodeRole::Modifier),
        ("abstract", SharedNodeRole::Modifier),
        ("static", SharedNodeRole::Modifier),
        ("final", SharedNodeRole::Modifier),
        ("strictfp", SharedNodeRole::Modifier),
        ("default", SharedNodeRole::Modifier),
        ("synchronized", SharedNodeRole::Modifier),
        ("native", SharedNodeRole::Modifier),
        ("transient", SharedNodeRole::Modifier),
        ("volatile", SharedNodeRole::Modifier),
        ("sealed", SharedNodeRole::Modifier),
        ("non-sealed", SharedNodeRole::Modifier),
        ("false", SharedNodeRole::BooleanLiteral),
        ("hex_integer_literal", SharedNodeRole::NumberLiteral),
        ("octal_integer_literal", SharedNodeRole::NumberLiteral),
        ("binary_integer_literal", SharedNodeRole::NumberLiteral),
        (
            "decimal_floating_point_literal",
            SharedNodeRole::NumberLiteral,
        ),
        ("hex_floating_point_literal", SharedNodeRole::NumberLiteral),
    ];

    for (node_type, expected) in jdt_aliases {
        assert_eq!(jdt_node_role(node_type), Some(expected));
    }
    for (node_type, expected) in tree_sitter_aliases {
        assert_eq!(tree_sitter_java_node_role(node_type), Some(expected));
    }
}

#[test]
fn unsupported_types_are_not_guessed_from_spelling() {
    for node_type in [
        "MethodInvocation",
        "InfixExpression",
        "QualifiedName",
        "SomeMethodDeclaration",
        "method_declaration",
        "",
    ] {
        assert_eq!(jdt_node_role(node_type), None, "JDT type {node_type}");
    }
    for node_type in [
        "method_invocation",
        "binary_expression",
        "scoped_identifier",
        "modifiers",
        "enum",
        "record",
        "custom_method_declaration",
        "MethodDeclaration",
        "",
    ] {
        assert_eq!(
            tree_sitter_java_node_role(node_type),
            None,
            "tree-sitter type {node_type}"
        );
    }
}

#[test]
fn keyword_roles_are_backed_by_grammar_tokens_not_the_modifiers_container() {
    let node_types: Vec<serde_json::Value> =
        serde_json::from_str(tree_sitter_java::NODE_TYPES).unwrap();
    let tokens = [
        "class",
        "interface",
        "public",
        "private",
        "protected",
        "abstract",
        "static",
        "final",
        "strictfp",
        "default",
        "synchronized",
        "native",
        "transient",
        "volatile",
        "sealed",
        "non-sealed",
    ];

    for token in tokens {
        assert!(
            node_types
                .iter()
                .any(|entry| entry["type"] == token && entry["named"] == false),
            "tree-sitter-java grammar does not declare token {token}"
        );
    }
    assert!(
        node_types
            .iter()
            .any(|entry| entry["type"] == "modifiers" && entry["named"] == true)
    );
    assert_eq!(tree_sitter_java_node_role("modifiers"), None);
}

#[test]
fn parses_a_god_info_mapping() {
    let mapping = parse_god_info("MethodDeclaration[12-34]:SimpleName[56-60]").unwrap();

    assert_eq!(
        mapping,
        JdtOracleMapping {
            before: JdtOracleNode {
                node_type: "MethodDeclaration".to_owned(),
                utf16_code_units: OffsetRange { start: 12, end: 34 },
            },
            after: JdtOracleNode {
                node_type: "SimpleName".to_owned(),
                utf16_code_units: OffsetRange { start: 56, end: 60 },
            },
        }
    );
}

#[test]
fn malformed_god_info_is_rejected() {
    for info in [
        "MethodDeclaration[1-2]",
        "MethodDeclaration[1-2]:SimpleName[3-4]:Name[5-6]",
        "[1-2]:SimpleName[3-4]",
        "Method Declaration[1-2]:SimpleName[3-4]",
        "MethodDeclaration[1]:SimpleName[3-4]",
        "MethodDeclaration[-1-2]:SimpleName[3-4]",
        "MethodDeclaration[2-1]:SimpleName[3-4]",
        "MethodDeclaration[1-2]:SimpleName[3-4",
        " MethodDeclaration[1-2]:SimpleName[3-4]",
        "MethodDeclaration[1-2]:SimpleName[3-4] ",
        "MethodDeclaration[184467440737095516160-184467440737095516161]:SimpleName[3-4]",
    ] {
        assert!(
            parse_god_info(info).is_err(),
            "accepted malformed info: {info}"
        );
    }
}

#[test]
fn converts_utf16_boundaries_to_utf8_byte_offsets() {
    let source = "aé中😀z";
    let expected = [(0, 0), (1, 1), (2, 3), (3, 6), (5, 10), (6, 11)];

    for (utf16, utf8) in expected {
        assert_eq!(utf16_offset_to_byte_offset(source, utf16).unwrap(), utf8);
    }
}

#[test]
fn rejects_surrogate_interior_and_out_of_bounds_offsets() {
    let source = "a😀z";

    assert!(utf16_offset_to_byte_offset(source, 2).is_err());
    assert!(utf16_offset_to_byte_offset(source, 5).is_err());
}

#[test]
fn normalizes_non_bmp_ranges_on_both_sides() {
    let mapping = parse_oracle_mapping("SimpleName[1-3]:SimpleName[0-2]", "A😀BC", "éx").unwrap();

    assert_eq!(
        mapping,
        OracleMapping {
            before: OracleNode {
                node_type: "SimpleName".to_owned(),
                role: SharedNodeRole::SimpleName,
                jdt_utf16_code_units: OffsetRange { start: 1, end: 3 },
                utf8_bytes: OffsetRange { start: 1, end: 5 },
            },
            after: OracleNode {
                node_type: "SimpleName".to_owned(),
                role: SharedNodeRole::SimpleName,
                jdt_utf16_code_units: OffsetRange { start: 0, end: 2 },
                utf8_bytes: OffsetRange { start: 0, end: 3 },
            },
        }
    );
}

#[test]
fn conversion_rejects_invalid_range_boundaries() {
    assert!(parse_oracle_mapping("SimpleName[2-3]:SimpleName[0-1]", "A😀", "x").is_err());
    assert!(parse_oracle_mapping("SimpleName[1-3]:SimpleName[0-2]", "A😀", "x").is_err());

    let reversed = JdtOracleMapping {
        before: JdtOracleNode {
            node_type: "Name".to_owned(),
            utf16_code_units: OffsetRange { start: 2, end: 1 },
        },
        after: JdtOracleNode {
            node_type: "Name".to_owned(),
            utf16_code_units: OffsetRange { start: 0, end: 1 },
        },
    };
    assert!(normalize_oracle_mapping(&reversed, "ab", "x").is_err());
}

#[test]
fn accepts_empty_ranges_at_valid_boundaries() {
    let mapping = parse_oracle_mapping("SimpleName[3-3]:SimpleName[0-0]", "A😀", "").unwrap();

    assert_eq!(mapping.before.utf8_bytes, OffsetRange { start: 5, end: 5 });
    assert_eq!(mapping.after.utf8_bytes, OffsetRange { start: 0, end: 0 });
}

#[test]
fn normalized_mapping_has_a_stable_serialized_shape() {
    let mapping = parse_oracle_mapping("SimpleName[1-3]:SimpleName[0-2]", "A😀", "éx").unwrap();
    let value = serde_json::to_value(&mapping).unwrap();

    assert_eq!(value["before"]["node_type"], "SimpleName");
    assert_eq!(value["before"]["role"], "simple_name");
    assert_eq!(value["before"]["jdt_utf16_code_units"]["start"], 1);
    assert_eq!(value["before"]["utf8_bytes"]["end"], 5);
    assert_eq!(
        serde_json::from_value::<OracleMapping>(value).unwrap(),
        mapping
    );
}

#[test]
fn normalization_rejects_unsupported_jdt_types() {
    let error = parse_oracle_mapping("MethodInvocation[0-1]:SimpleName[0-1]", "x", "x")
        .unwrap_err()
        .to_string();

    assert!(error.contains("unsupported DiffBenchmark before JDT node type MethodInvocation"));
}

#[test]
fn declaration_resolution_removes_only_leading_java_trivia() {
    let source =
        "/** docs */\npublic class Demo {\n  /** method */\n  public static void run() {}\n}\n";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();
    let class_end = source.rfind('}').unwrap() + 1;
    let class_node = JdtOracleNode {
        node_type: "TypeDeclaration".to_owned(),
        utf16_code_units: OffsetRange {
            start: 0,
            end: class_end,
        },
    };
    assert_eq!(
        resolve_jdt_node(&class_node, source, &nodes).unwrap(),
        Some(ComparableNode {
            role: SharedNodeRole::TypeDeclaration,
            utf8_bytes: OffsetRange {
                start: source.find("public class").unwrap(),
                end: class_end,
            },
        })
    );

    let method_comment = source.find("/** method */").unwrap();
    let method_start = source.find("public static").unwrap();
    let method_end = source.find("run() {}").unwrap() + "run() {}".len();
    let method_node = JdtOracleNode {
        node_type: "MethodDeclaration".to_owned(),
        utf16_code_units: OffsetRange {
            start: method_comment,
            end: method_end,
        },
    };
    assert_eq!(
        resolve_jdt_node(&method_node, source, &nodes).unwrap(),
        Some(ComparableNode {
            role: SharedNodeRole::MethodDeclaration,
            utf8_bytes: OffsetRange {
                start: method_start,
                end: method_end,
            },
        })
    );
}

#[test]
fn modifier_container_is_excluded_but_individual_keywords_are_comparable() {
    let source = "public static class Demo {}";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();
    let modifiers: Vec<_> = nodes
        .iter()
        .filter(|node| node.role == SharedNodeRole::Modifier)
        .copied()
        .collect();

    assert_eq!(
        modifiers,
        [
            ComparableNode {
                role: SharedNodeRole::Modifier,
                utf8_bytes: OffsetRange { start: 0, end: 6 },
            },
            ComparableNode {
                role: SharedNodeRole::Modifier,
                utf8_bytes: OffsetRange { start: 7, end: 13 },
            },
        ]
    );
}

#[test]
fn oracle_resolution_uses_utf16_offsets_against_unicode_source() {
    let source = "class 演示 { int 值; }";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();
    let value_start = source.find('值').unwrap();
    let value_end = value_start + '值'.len_utf8();
    let oracle = JdtOracleNode {
        node_type: "SimpleName".to_owned(),
        utf16_code_units: OffsetRange {
            start: source[..value_start].encode_utf16().count(),
            end: source[..value_end].encode_utf16().count(),
        },
    };

    assert_eq!(
        resolve_jdt_node(&oracle, source, &nodes).unwrap(),
        Some(ComparableNode {
            role: SharedNodeRole::SimpleName,
            utf8_bytes: OffsetRange {
                start: value_start,
                end: value_end,
            },
        })
    );
}
