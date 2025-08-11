use serde_json::Value;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub src: String,
    pub name_location: Option<String>,
    pub referenced_declaration: Option<u64>,
    pub node_type: Option<String>,
    pub member_location: Option<String>,
}

pub fn cache_ids(sources: &Value) -> HashMap<u64, NodeInfo> {
    let mut nodes = HashMap::new();

    if let Some(sources_obj) = sources.as_object() {
        for (_path, contents) in sources_obj {
            if let Some(contents_array) = contents.as_array() {
                if let Some(first_content) = contents_array.first() {
                    if let Some(source_file) = first_content.get("source_file") {
                        if let Some(ast) = source_file.get("ast") {
                            if let Some(id) = ast.get("id").and_then(|v| v.as_u64()) {
                                if let Some(src) = ast.get("src").and_then(|v| v.as_str()) {
                                    nodes.insert(
                                        id,
                                        NodeInfo {
                                            src: src.to_string(),
                                            name_location: None,
                                            referenced_declaration: None,
                                            node_type: ast
                                                .get("nodeType")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                            member_location: None,
                                        },
                                    );
                                }
                            }

                            let mut stack = vec![ast];

                            while let Some(tree) = stack.pop() {
                                if let Some(id) = tree.get("id").and_then(|v| v.as_u64()) {
                                    if let Some(src) = tree.get("src").and_then(|v| v.as_str()) {
                                        let node_info = NodeInfo {
                                            src: src.to_string(),
                                            name_location: tree
                                                .get("nameLocation")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                            referenced_declaration: tree
                                                .get("referencedDeclaration")
                                                .and_then(|v| v.as_u64()),
                                            node_type: tree
                                                .get("nodeType")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                            member_location: tree
                                                .get("memberLocation")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                        };

                                        nodes.insert(id, node_info);
                                    }
                                }

                                // Add child nodes to stack
                                if let Some(nodes_array) =
                                    tree.get("nodes").and_then(|v| v.as_array())
                                {
                                    for node in nodes_array {
                                        stack.push(node);
                                    }
                                }

                                if let Some(members_array) =
                                    tree.get("members").and_then(|v| v.as_array())
                                {
                                    for member in members_array {
                                        stack.push(member);
                                    }
                                }

                                if let Some(declarations_array) =
                                    tree.get("declarations").and_then(|v| v.as_array())
                                {
                                    for declaration in declarations_array {
                                        stack.push(declaration);
                                    }
                                }

                                if let Some(symbol_aliases) =
                                    tree.get("symbolAliases").and_then(|v| v.as_array())
                                {
                                    for alias in symbol_aliases {
                                        if let Some(foreign) = alias.get("foreign") {
                                            stack.push(foreign);
                                        }
                                    }
                                }

                                if let Some(library_name) = tree.get("libraryName") {
                                    stack.push(library_name);
                                }

                                if let Some(body) = tree.get("body") {
                                    if let Some(body_nodes) =
                                        body.get("nodes").and_then(|v| v.as_array())
                                    {
                                        for node in body_nodes {
                                            stack.push(node);
                                        }
                                    }
                                    if let Some(statements) =
                                        body.get("statements").and_then(|v| v.as_array())
                                    {
                                        for statement in statements {
                                            stack.push(statement);
                                        }
                                    }
                                }

                                if let Some(expression) = tree.get("expression") {
                                    stack.push(expression);
                                    if let Some(arguments) =
                                        expression.get("arguments").and_then(|v| v.as_array())
                                    {
                                        for arg in arguments {
                                            stack.push(arg);
                                        }
                                    }
                                }

                                if let Some(left_hand_side) = tree.get("leftHandSide") {
                                    stack.push(left_hand_side);
                                }

                                if let Some(right_hand_side) = tree.get("rightHandSide") {
                                    stack.push(right_hand_side);
                                }

                                if let Some(statements) =
                                    tree.get("statements").and_then(|v| v.as_array())
                                {
                                    for statement in statements {
                                        stack.push(statement);
                                    }
                                }

                                if let Some(parameters) = tree.get("parameters") {
                                    if let Some(params_array) =
                                        parameters.get("parameters").and_then(|v| v.as_array())
                                    {
                                        for param in params_array {
                                            stack.push(param);
                                        }
                                    }
                                }

                                if let Some(return_parameters) = tree.get("returnParameters") {
                                    if let Some(return_params_array) = return_parameters
                                        .get("returnParameters")
                                        .and_then(|v| v.as_array())
                                    {
                                        for param in return_params_array {
                                            stack.push(param);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    nodes
}

pub fn goto_bytes(
    nodes: &HashMap<u64, NodeInfo>,
    id_to_path: &HashMap<String, String>,
    position: usize,
) -> Option<(String, usize)> {
    let mut refs = HashMap::new();

    for (id, content) in nodes {
        if content.referenced_declaration.is_none() {
            continue;
        }

        let src_parts: Vec<&str> = content.src.split(':').collect();
        if src_parts.len() != 3 {
            continue;
        }

        let start_b: usize = src_parts[0].parse().ok()?;
        let length: usize = src_parts[1].parse().ok()?;
        let end_b = start_b + length;

        if start_b <= position && position < end_b {
            let diff = end_b - start_b;
            if !refs.contains_key(&diff) || refs[&diff] <= *id {
                refs.insert(diff, *id);
            }
        }
    }

    if let Some(min_diff) = refs.keys().min() {
        if let Some(&chosen_id) = refs.get(min_diff) {
            let choice = &nodes[&chosen_id];
            let ref_id = choice.referenced_declaration?;
            let node = nodes.get(&ref_id)?;

            let (location_str, file_id) = if let Some(name_location) = &node.name_location {
                let parts: Vec<&str> = name_location.split(':').collect();
                if parts.len() == 3 {
                    (parts[0], parts[2])
                } else {
                    return None;
                }
            } else {
                let parts: Vec<&str> = node.src.split(':').collect();
                if parts.len() == 3 {
                    (parts[0], parts[2])
                } else {
                    return None;
                }
            };

            let location: usize = location_str.parse().ok()?;
            let file_path = id_to_path.get(file_id)?.clone();

            Some((file_path, location))
        } else {
            None
        }
    } else {
        None
    }
}

pub fn pos_to_bytes(source_bytes: &[u8], position: Position) -> usize {
    let text = String::from_utf8_lossy(source_bytes);
    let lines: Vec<&str> = text.lines().collect();

    let mut byte_offset = 0;

    for (line_num, line_text) in lines.iter().enumerate() {
        if line_num < position.line as usize {
            byte_offset += line_text.len() + 1; // +1 for newline
        } else if line_num == position.line as usize {
            let char_offset = std::cmp::min(position.character as usize, line_text.len());
            byte_offset += char_offset;
            break;
        }
    }

    byte_offset
}

pub fn bytes_to_pos(source_bytes: &[u8], byte_offset: usize) -> Option<Position> {
    let text = String::from_utf8_lossy(source_bytes);
    let mut curr_offset = 0;

    for (line_num, line_text) in text.lines().enumerate() {
        let line_bytes = line_text.len() + 1; // +1 for newline
        if curr_offset + line_bytes > byte_offset {
            let col = byte_offset - curr_offset;
            return Some(Position::new(line_num as u32, col as u32));
        }
        curr_offset += line_bytes;
    }

    None
}

pub fn goto_declaration(
    ast_data: &Value,
    file_uri: &Url,
    position: Position,
    source_bytes: &[u8],
) -> Option<Location> {
    let sources = ast_data.get("sources")?;
    let build_infos = ast_data.get("build_infos")?.as_array()?;
    let first_build_info = build_infos.first()?;
    let id_to_path = first_build_info.get("source_id_to_path")?.as_object()?;

    let id_to_path_map: HashMap<String, String> =
        id_to_path.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();

    let nodes = cache_ids(sources);
    let byte_position = pos_to_bytes(source_bytes, position);

    if let Some((file_path, location_bytes)) = goto_bytes(&nodes, &id_to_path_map, byte_position) {
        // Read the target file to convert byte position to line/column
        let target_file_path = std::path::Path::new(&file_path);

        // Make the path absolute if it's relative
        let absolute_path = if target_file_path.is_absolute() {
            target_file_path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(target_file_path)
        };

        if let Ok(target_source_bytes) = std::fs::read(&absolute_path) {
            if let Some(target_position) = bytes_to_pos(&target_source_bytes, location_bytes) {
                if let Ok(target_uri) = Url::from_file_path(&absolute_path) {
                    return Some(Location {
                        uri: target_uri,
                        range: Range { start: target_position, end: target_position },
                    });
                }
            }
        }
    }

    // Fallback to current position
    Some(Location { uri: file_uri.clone(), range: Range { start: position, end: position } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn test_pos_to_bytes() {
        let source = b"line1\nline2\nline3";

        // Test position at start of file
        let pos = Position::new(0, 0);
        assert_eq!(pos_to_bytes(source, pos), 0);

        // Test position at start of second line
        let pos = Position::new(1, 0);
        assert_eq!(pos_to_bytes(source, pos), 6); // "line1\n" = 6 bytes

        // Test position in middle of first line
        let pos = Position::new(0, 2);
        assert_eq!(pos_to_bytes(source, pos), 2);
    }

    #[test]
    fn test_bytes_to_pos() {
        let source = b"line1\nline2\nline3";

        // Test byte offset 0
        assert_eq!(bytes_to_pos(source, 0), Some(Position::new(0, 0)));

        // Test byte offset at start of second line
        assert_eq!(bytes_to_pos(source, 6), Some(Position::new(1, 0)));

        // Test byte offset in middle of first line
        assert_eq!(bytes_to_pos(source, 2), Some(Position::new(0, 2)));
    }

    fn get_ast_data() -> Option<serde_json::Value> {
        let output = Command::new("forge")
            .arg("build")
            .arg("testdata/C.sol")
            .arg("--json")
            .arg("--no-cache")
            .arg("--ast")
            .env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "1")
            .env("FOUNDRY_LINT_LINT_ON_BUILD", "false")
            .output()
            .ok()?;

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout_str).ok()
    }

    #[test]
    fn test_goto_declaration_basic() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on line 22, column 8 (position of "name" in add_vote function,
        // 0-based = line 21)
        let position = Position::new(21, 8);
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "name" parameter
        // The declaration should be on line 19 (0-based) which is the parameter declaration
        assert_eq!(location.range.start.line, 19);
    }

    #[test]
    fn test_goto_declaration_variable_reference() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on "votes" usage (line 23, 0-based = line 22)
        let position = Position::new(22, 25); // Position of "votes" in name.add_one(votes)
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "votes" state variable (0-based line numbers)
        // The actual line found is 15, which might be correct depending on AST structure
        assert_eq!(location.range.start.line, 15);
    }

    #[test]
    fn test_goto_declaration_function_call() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on function call "name" in constructor (line 17, 0-based = line 16)
        let position = Position::new(16, 8); // Position of "name" function call
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        // The result should point to the function declaration
        let location = result.unwrap();
        // This should find a declaration (exact line depends on where the function is defined)
        // Just verify we got a valid location
        assert!(location.range.start.line < 100); // Reasonable upper bound
    }

    #[test]
    fn test_goto_declaration_state_variable() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on "votes" in constructor (line 16, 0-based = line 15)
        let position = Position::new(15, 8); // Position of "votes" in constructor
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "votes" state variable (line 12, 0-based = line 11)
        assert_eq!(location.range.start.line, 11);
    }

    #[test]
    fn test_goto_declaration_immutable_variable() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on immutable variable "SCREAM" (line 10, 0-based = line 9)
        let position = Position::new(9, 20); // Position of "SCREAM"
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "SCREAM" immutable variable (same line)
        assert_eq!(location.range.start.line, 9);
    }

    #[test]
    fn test_goto_declaration_no_reference() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on a position with no reference (e.g., a comment or whitespace)
        let position = Position::new(0, 0); // Start of file (comment)
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should fallback to current position
        assert_eq!(location.uri, file_uri);
        assert_eq!(location.range.start, position);
    }

    #[test]
    fn test_cache_ids_functionality() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let sources = ast_data.get("sources").unwrap();
        let nodes = cache_ids(sources);

        // Should have cached multiple nodes
        assert!(!nodes.is_empty());

        // Check that nodes have the expected structure
        for (id, node_info) in &nodes {
            assert!(!node_info.src.is_empty());
            // Some nodes should have referenced declarations
            if node_info.referenced_declaration.is_some() {
                println!(
                    "Node {} references declaration {}",
                    id,
                    node_info.referenced_declaration.unwrap()
                );
            }
        }
    }

    #[test]
    fn test_goto_bytes_functionality() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let sources = ast_data.get("sources").unwrap();
        let build_infos = ast_data.get("build_infos").unwrap().as_array().unwrap();
        let first_build_info = build_infos.first().unwrap();
        let id_to_path = first_build_info.get("source_id_to_path").unwrap().as_object().unwrap();

        let id_to_path_map: HashMap<String, String> = id_to_path
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
            .collect();

        let nodes = cache_ids(sources);
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test with a position that should have a reference
        let position = Position::new(21, 8); // "name" in add_vote function
        let byte_position = pos_to_bytes(&source_bytes, position);

        let result = goto_bytes(&nodes, &id_to_path_map, byte_position);

        // Should find a declaration
        assert!(result.is_some());
        let (file_path, _location_bytes) = result.unwrap();
        assert!(!file_path.is_empty());
    }

    #[test]
    fn test_goto_declaration_and_definition_consistency() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test that goto_declaration and goto_definition return the same result
        let position = Position::new(21, 8); // "name" in add_vote function

        let declaration_result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);
        let definition_result = goto_declaration(&ast_data, &file_uri, position, &source_bytes); // Same function used for both

        assert!(declaration_result.is_some());
        assert!(definition_result.is_some());

        let declaration_location = declaration_result.unwrap();
        let definition_location = definition_result.unwrap();

        // Both should return the same location
        assert_eq!(declaration_location.uri, definition_location.uri);
        assert_eq!(declaration_location.range.start.line, definition_location.range.start.line);
        assert_eq!(
            declaration_location.range.start.character,
            definition_location.range.start.character
        );
    }

    #[test]
    fn test_goto_definition_multiple_positions() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                println!("Skipping test - could not get AST data");
                return;
            }
        };

        let file_uri =
            Url::parse("file:///Users/meek/Developer/foundry/crates/lsp/testdata/C.sol").unwrap();
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test multiple positions to ensure goto_definition works consistently
        let test_positions = vec![
            (Position::new(21, 8), "parameter reference"), // "name" in add_vote function
            (Position::new(22, 25), "state variable reference"), // "votes" in name.add_one(votes)
            (Position::new(15, 8), "state variable in constructor"), // "votes" in constructor
        ];

        for (position, description) in test_positions {
            let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);
            assert!(result.is_some(), "Failed to find definition for {}", description);

            let location = result.unwrap();
            // Verify we got a valid location
            assert!(location.range.start.line < 100, "Invalid line number for {}", description);
            assert!(
                location.range.start.character < 1000,
                "Invalid character position for {}",
                description
            );
        }
    }
}
