use stratadiff::diffbenchmark::{
    ComparableNode, GodReport, JdtOracleMapping, JdtOracleNode, OffsetRange, OracleMapping,
    OracleNode, SharedNodeRole, comparable_tree_sitter_java_nodes, jdt_node_role,
    normalize_oracle_mapping, parse_god_info, parse_god_report, parse_oracle_mapping,
    resolve_jdt_node, tree_sitter_java_node_role, tree_sitter_java_node_roles,
    utf16_offset_to_byte_offset,
};

fn assert_resolves_fragment(
    source: &str,
    candidates: &[ComparableNode],
    node_type: &str,
    fragment: &str,
) {
    let start = source
        .find(fragment)
        .unwrap_or_else(|| panic!("missing source fragment {fragment:?}"));
    assert_resolves_range(
        source,
        candidates,
        node_type,
        OffsetRange {
            start,
            end: start + fragment.len(),
        },
    );
}

fn assert_resolves_range(
    source: &str,
    candidates: &[ComparableNode],
    node_type: &str,
    utf8_bytes: OffsetRange,
) {
    let oracle = JdtOracleNode {
        node_type: node_type.to_owned(),
        utf16_code_units: OffsetRange {
            start: source[..utf8_bytes.start].encode_utf16().count(),
            end: source[..utf8_bytes.end].encode_utf16().count(),
        },
    };
    assert_eq!(
        resolve_jdt_node(&oracle, source, candidates).unwrap(),
        Some(ComparableNode {
            role: jdt_node_role(node_type).unwrap(),
            utf8_bytes,
        }),
        "failed to resolve {node_type}"
    );
}

fn assert_does_not_resolve_fragment(
    source: &str,
    candidates: &[ComparableNode],
    node_type: &str,
    fragment: &str,
) {
    let start = source
        .find(fragment)
        .unwrap_or_else(|| panic!("missing source fragment {fragment:?}"));
    let oracle = JdtOracleNode {
        node_type: node_type.to_owned(),
        utf16_code_units: OffsetRange {
            start: source[..start].encode_utf16().count(),
            end: source[..start + fragment.len()].encode_utf16().count(),
        },
    };
    assert_eq!(
        resolve_jdt_node(&oracle, source, candidates).unwrap(),
        None,
        "unexpectedly resolved {node_type}"
    );
}

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
        (
            "SingleVariableDeclaration",
            "formal_parameter",
            SharedNodeRole::SingleVariableDeclaration,
        ),
        (
            "MarkerAnnotation",
            "marker_annotation",
            SharedNodeRole::MarkerAnnotation,
        ),
        ("SwitchCase", "switch_label", SharedNodeRole::SwitchCase),
        (
            "TypeParameter",
            "type_parameter",
            SharedNodeRole::TypeParameter,
        ),
        (
            "VariableDeclarationFragment",
            "variable_declarator",
            SharedNodeRole::VariableDeclarationFragment,
        ),
        (
            "MethodInvocation",
            "method_invocation",
            SharedNodeRole::MethodInvocation,
        ),
        ("ArrayAccess", "array_access", SharedNodeRole::ArrayAccess),
        (
            "ArrayCreation",
            "array_creation_expression",
            SharedNodeRole::ArrayCreation,
        ),
        (
            "CastExpression",
            "cast_expression",
            SharedNodeRole::CastExpression,
        ),
        (
            "ClassInstanceCreation",
            "object_creation_expression",
            SharedNodeRole::ClassInstanceCreation,
        ),
        (
            "ConditionalExpression",
            "ternary_expression",
            SharedNodeRole::ConditionalExpression,
        ),
        (
            "InfixExpression",
            "binary_expression",
            SharedNodeRole::InfixExpression,
        ),
        (
            "InstanceofExpression",
            "instanceof_expression",
            SharedNodeRole::InstanceofExpression,
        ),
        (
            "ParenthesizedExpression",
            "parenthesized_expression",
            SharedNodeRole::ParenthesizedExpression,
        ),
        (
            "PrefixExpression",
            "unary_expression",
            SharedNodeRole::PrefixExpression,
        ),
        ("ThisExpression", "this", SharedNodeRole::ThisExpression),
        ("LineComment", "line_comment", SharedNodeRole::LineComment),
        (
            "BlockComment",
            "block_comment",
            SharedNodeRole::BlockComment,
        ),
        (
            "QualifiedName",
            "scoped_identifier",
            SharedNodeRole::QualifiedAccess,
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
fn type_declaration_kind_requires_declaration_context() {
    let source = "class Demo { Class<?> literal = Foo.class; }\ninterface Example {}";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();

    assert_eq!(
        jdt_node_role("TYPE_DECLARATION_KIND"),
        Some(SharedNodeRole::TypeDeclarationKind)
    );
    assert_eq!(tree_sitter_java_node_role("class"), None);
    assert_eq!(tree_sitter_java_node_role("interface"), None);
    assert_resolves_fragment(source, &nodes, "TYPE_DECLARATION_KIND", "class");
    assert_resolves_fragment(source, &nodes, "TYPE_DECLARATION_KIND", "interface");
    assert_resolves_fragment(source, &nodes, "TypeLiteral", "Foo.class");

    let kind_ranges: Vec<_> = nodes
        .iter()
        .filter(|node| node.role == SharedNodeRole::TypeDeclarationKind)
        .map(|node| node.utf8_bytes)
        .collect();
    let interface_start = source.find("interface").unwrap();
    assert_eq!(
        kind_ranges,
        [
            OffsetRange {
                start: 0,
                end: "class".len(),
            },
            OffsetRange {
                start: interface_start,
                end: interface_start + "interface".len(),
            },
        ]
    );
}

#[test]
fn type_identifiers_cover_the_nested_jdt_type_and_name_nodes() {
    let source = "class Demo { java.util.List<String> value; }";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();

    assert_resolves_fragment(source, &nodes, "SimpleName", "Demo");
    assert_resolves_fragment(source, &nodes, "SimpleType", "java.util.List");
    assert_resolves_fragment(source, &nodes, "QualifiedName", "java.util.List");
    assert_resolves_fragment(source, &nodes, "SimpleType", "String");
    assert_resolves_fragment(source, &nodes, "SimpleName", "String");
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
        ("FieldAccess", SharedNodeRole::QualifiedAccess),
        (
            "INFIX_EXPRESSION_OPERATOR",
            SharedNodeRole::InfixExpressionOperator,
        ),
        (
            "METHOD_INVOCATION_ARGUMENTS",
            SharedNodeRole::MethodInvocationArguments,
        ),
        (
            "METHOD_INVOCATION_RECEIVER",
            SharedNodeRole::MethodInvocationReceiver,
        ),
    ];
    let tree_sitter_aliases = [
        ("constructor_declaration", SharedNodeRole::MethodDeclaration),
        (
            "compact_constructor_declaration",
            SharedNodeRole::MethodDeclaration,
        ),
        ("constant_declaration", SharedNodeRole::FieldDeclaration),
        ("interface_declaration", SharedNodeRole::TypeDeclaration),
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
        (
            "catch_formal_parameter",
            SharedNodeRole::SingleVariableDeclaration,
        ),
        (
            "spread_parameter",
            SharedNodeRole::SingleVariableDeclaration,
        ),
        ("field_access", SharedNodeRole::QualifiedAccess),
    ];

    for (node_type, expected) in jdt_aliases {
        assert_eq!(jdt_node_role(node_type), Some(expected));
    }
    for (node_type, expected) in tree_sitter_aliases {
        assert_eq!(tree_sitter_java_node_role(node_type), Some(expected));
    }
}

#[test]
fn annotation_kinds_expose_both_contextual_roles() {
    assert_eq!(
        tree_sitter_java_node_roles("annotation"),
        [
            SharedNodeRole::NormalAnnotation,
            SharedNodeRole::SingleMemberAnnotation,
        ]
    );
    assert_eq!(tree_sitter_java_node_role("annotation"), None);
    assert_eq!(
        jdt_node_role("NormalAnnotation"),
        Some(SharedNodeRole::NormalAnnotation)
    );
    assert_eq!(
        jdt_node_role("SingleMemberAnnotation"),
        Some(SharedNodeRole::SingleMemberAnnotation)
    );
}

#[test]
fn unsupported_types_are_not_guessed_from_spelling() {
    for node_type in [
        "LambdaExpression",
        "PostfixExpression",
        "SuperFieldAccess",
        "SomeMethodDeclaration",
        "method_declaration",
        "",
    ] {
        assert_eq!(jdt_node_role(node_type), None, "JDT type {node_type}");
    }
    for node_type in [
        "lambda_expression",
        "update_expression",
        "scoped_type_identifier",
        "annotation",
        "argument_list",
        "class",
        "interface",
        "receiver_parameter",
        "+",
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
fn expanded_roles_are_backed_by_tree_sitter_java_node_types() {
    let node_types: Vec<serde_json::Value> =
        serde_json::from_str(tree_sitter_java::NODE_TYPES).unwrap();
    let named_types = [
        "formal_parameter",
        "catch_formal_parameter",
        "spread_parameter",
        "marker_annotation",
        "annotation",
        "switch_label",
        "type_parameter",
        "variable_declarator",
        "method_invocation",
        "array_access",
        "array_creation_expression",
        "cast_expression",
        "object_creation_expression",
        "ternary_expression",
        "binary_expression",
        "instanceof_expression",
        "parenthesized_expression",
        "unary_expression",
        "this",
        "line_comment",
        "block_comment",
        "scoped_identifier",
        "field_access",
        "argument_list",
    ];

    for node_type in named_types {
        assert!(
            node_types
                .iter()
                .any(|entry| entry["type"] == node_type && entry["named"] == true),
            "tree-sitter-java grammar does not declare named node {node_type}"
        );
    }

    let binary_expression = node_types
        .iter()
        .find(|entry| entry["type"] == "binary_expression" && entry["named"] == true)
        .unwrap();
    assert!(
        binary_expression["fields"]["operator"]["types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["type"] == "+" && entry["named"] == false)
    );
    assert!(
        node_types
            .iter()
            .any(|entry| entry["type"] == ";" && entry["named"] == false)
    );

    let invocation = node_types
        .iter()
        .find(|entry| entry["type"] == "method_invocation" && entry["named"] == true)
        .unwrap();
    assert_eq!(
        invocation["fields"]["arguments"]["types"][0]["type"],
        "argument_list"
    );
    assert!(invocation["fields"]["object"]["required"] == false);
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
    let error = parse_oracle_mapping("LambdaExpression[0-1]:SimpleName[0-1]", "x", "x")
        .unwrap_err()
        .to_string();

    assert!(error.contains("unsupported DiffBenchmark before JDT node type LambdaExpression"));
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
fn declaration_resolution_never_strips_non_javadoc_trivia() {
    let source = "// line\n/* block */\npublic class Demo {}";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();
    let class = JdtOracleNode {
        node_type: "TypeDeclaration".to_owned(),
        utf16_code_units: OffsetRange {
            start: 0,
            end: source.len(),
        },
    };

    assert_eq!(resolve_jdt_node(&class, source, &nodes).unwrap(), None);

    let whitespace_source = "  public class Demo {}";
    let whitespace_nodes = comparable_tree_sitter_java_nodes(whitespace_source.as_bytes()).unwrap();
    let whitespace_class = JdtOracleNode {
        node_type: "TypeDeclaration".to_owned(),
        utf16_code_units: OffsetRange {
            start: 0,
            end: whitespace_source.len(),
        },
    };
    assert_eq!(
        resolve_jdt_node(&whitespace_class, whitespace_source, &whitespace_nodes).unwrap(),
        None
    );
}

#[test]
fn expanded_taxonomy_resolves_exact_grammar_nodes() {
    let source = r#"import java.util.List;
/* block comment */
// line comment
@Marker
@Normal(key = "value")
@Single("value")
class Demo<Type> {
    Object field;
    void receive(Demo this) {}
    void run(String formal, String... spread) {
        try {} catch (Exception caught) {}
        int fragment = 1;
        serviceReceiver.call(firstArgument, secondArgument);
        Object access = array[index];
        Object madeArray = new int[1];
        Object cast = (String) value;
        Object made = new Demo();
        Object conditional = flag ? first : second;
        ;
        Object infix = leftOperand != rightOperand;
        boolean checked = value instanceof String;
        Object grouped = (groupedValue);
        boolean negated = !flag;
        ++prefixCounter;
        postfixCounter++;
        Object qualified = packageName.member;
        Object fieldAccess = this.field;
        switch (fragment) {
            case 1:
                break;
            default:
                break;
        }
    }
}
"#;
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();

    for (node_type, fragment) in [
        ("QualifiedName", "java.util.List"),
        ("BlockComment", "/* block comment */"),
        ("LineComment", "// line comment"),
        ("MarkerAnnotation", "@Marker"),
        ("NormalAnnotation", "@Normal(key = \"value\")"),
        ("SingleMemberAnnotation", "@Single(\"value\")"),
        ("TypeParameter", "Type"),
        ("SingleVariableDeclaration", "String formal"),
        ("SingleVariableDeclaration", "String... spread"),
        ("SingleVariableDeclaration", "Exception caught"),
        ("VariableDeclarationFragment", "fragment = 1"),
        (
            "MethodInvocation",
            "serviceReceiver.call(firstArgument, secondArgument)",
        ),
        ("ArrayAccess", "array[index]"),
        ("ArrayCreation", "new int[1]"),
        ("CastExpression", "(String) value"),
        ("ClassInstanceCreation", "new Demo()"),
        ("ConditionalExpression", "flag ? first : second"),
        ("InfixExpression", "leftOperand != rightOperand"),
        ("InstanceofExpression", "value instanceof String"),
        ("ParenthesizedExpression", "(groupedValue)"),
        ("PrefixExpression", "!flag"),
        ("PrefixExpression", "++prefixCounter"),
        ("FieldAccess", "this.field"),
        ("SwitchCase", "case 1:"),
    ] {
        assert_resolves_fragment(source, &nodes, node_type, fragment);
    }

    assert_does_not_resolve_fragment(source, &nodes, "SingleVariableDeclaration", "Demo this");
    let empty_start = source.find("        ;\n").unwrap() + "        ".len();
    assert_resolves_range(
        source,
        &nodes,
        "EmptyStatement",
        OffsetRange {
            start: empty_start,
            end: empty_start + 1,
        },
    );
    let this_start = source.find("this.field").unwrap();
    assert_resolves_range(
        source,
        &nodes,
        "ThisExpression",
        OffsetRange {
            start: this_start,
            end: this_start + "this".len(),
        },
    );
    let postfix_start = source.find("postfixCounter++").unwrap();
    let postfix = JdtOracleNode {
        node_type: "PrefixExpression".to_owned(),
        utf16_code_units: OffsetRange {
            start: postfix_start,
            end: postfix_start + "postfixCounter++".len(),
        },
    };
    assert_eq!(resolve_jdt_node(&postfix, source, &nodes).unwrap(), None);
}

#[test]
fn synthetic_roles_require_exact_tree_sitter_field_context() {
    let source = "class Demo extends Base { void run() { service.call(first, second); super.call(superArgument); Outer.super.call(qualifiedSuperArgument); boolean same = left == right; } }";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();

    assert_resolves_fragment(source, &nodes, "METHOD_INVOCATION_RECEIVER", "service");
    assert_resolves_fragment(
        source,
        &nodes,
        "METHOD_INVOCATION_ARGUMENTS",
        "first, second",
    );
    assert_resolves_fragment(source, &nodes, "INFIX_EXPRESSION_OPERATOR", "==");

    let receiver_start = source.find("service").unwrap();
    assert!(nodes.contains(&ComparableNode {
        role: SharedNodeRole::SimpleName,
        utf8_bytes: OffsetRange {
            start: receiver_start,
            end: receiver_start + "service".len(),
        },
    }));
    assert_eq!(tree_sitter_java_node_role("argument_list"), None);
    assert_eq!(tree_sitter_java_node_role("=="), None);

    for (node_type, fragment) in [
        ("MethodInvocation", "super.call(superArgument)"),
        ("METHOD_INVOCATION_ARGUMENTS", "superArgument"),
        (
            "MethodInvocation",
            "Outer.super.call(qualifiedSuperArgument)",
        ),
        ("METHOD_INVOCATION_RECEIVER", "Outer"),
        ("METHOD_INVOCATION_ARGUMENTS", "qualifiedSuperArgument"),
    ] {
        assert_does_not_resolve_fragment(source, &nodes, node_type, fragment);
    }
}

#[test]
fn infix_operator_roles_follow_jdt_extended_operand_structure() {
    fn operator_ranges(source: &str) -> Vec<OffsetRange> {
        comparable_tree_sitter_java_nodes(source.as_bytes())
            .unwrap()
            .into_iter()
            .filter(|node| node.role == SharedNodeRole::InfixExpressionOperator)
            .map(|node| node.utf8_bytes)
            .collect()
    }

    let homogeneous = "class Demo { int value = a + b + c; }";
    let homogeneous_operators: Vec<_> = homogeneous
        .match_indices('+')
        .map(|(start, operator)| OffsetRange {
            start,
            end: start + operator.len(),
        })
        .collect();
    assert_eq!(operator_ranges(homogeneous), [homogeneous_operators[0]]);

    let mixed = "class Demo { int value = a + b - c; }";
    let mixed_operators: Vec<_> = mixed
        .match_indices(['+', '-'])
        .map(|(start, operator)| OffsetRange {
            start,
            end: start + operator.len(),
        })
        .collect();
    assert_eq!(operator_ranges(mixed), mixed_operators);

    let parenthesized = "class Demo { int value = (a + b) + c; }";
    let parenthesized_operators: Vec<_> = parenthesized
        .match_indices('+')
        .map(|(start, operator)| OffsetRange {
            start,
            end: start + operator.len(),
        })
        .collect();
    assert_eq!(operator_ranges(parenthesized), parenthesized_operators);

    for (operator, equality) in [
        ("==", "class Demo { boolean value = a == b == c; }"),
        ("!=", "class Demo { boolean value = a != b != c; }"),
    ] {
        let equality_operators: Vec<_> = equality
            .match_indices(operator)
            .map(|(start, operator)| OffsetRange {
                start,
                end: start + operator.len(),
            })
            .collect();
        assert_eq!(operator_ranges(equality), equality_operators);
    }
}

#[test]
fn enhanced_for_variables_use_jdt_single_variable_declaration_ranges() {
    let source = "class Demo { void run() { for (String item : items) {} for (final String row[] : matrix) {} } }";
    let nodes = comparable_tree_sitter_java_nodes(source.as_bytes()).unwrap();

    assert_resolves_fragment(source, &nodes, "SingleVariableDeclaration", "String item");
    assert_resolves_fragment(
        source,
        &nodes,
        "SingleVariableDeclaration",
        "final String row[]",
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
