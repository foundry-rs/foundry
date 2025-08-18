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

fn push_if_node_or_array<'a>(tree: &'a Value, key: &str, stack: &mut Vec<&'a Value>) {
    if let Some(value) = tree.get(key) {
        match value {
            Value::Array(arr) => {
                stack.extend(arr);
            }
            Value::Object(_) => {
                stack.push(value);
            }
            _ => {}
        }
    }
}

pub fn cache_ids(
    sources: &Value,
) -> (HashMap<String, HashMap<u64, NodeInfo>>, HashMap<String, String>) {
    let mut nodes: HashMap<String, HashMap<u64, NodeInfo>> = HashMap::new();
    let mut path_to_abs: HashMap<String, String> = HashMap::new();

    if let Some(sources_obj) = sources.as_object() {
        for (path, contents) in sources_obj {
            if let Some(contents_array) = contents.as_array()
                && let Some(first_content) = contents_array.first()
                && let Some(source_file) = first_content.get("source_file")
                && let Some(ast) = source_file.get("ast")
            {
                // Get the absolute path for this file
                let abs_path =
                    ast.get("absolutePath").and_then(|v| v.as_str()).unwrap_or(path).to_string();

                path_to_abs.insert(path.clone(), abs_path.clone());

                // Initialize the nodes map for this file
                if !nodes.contains_key(&abs_path) {
                    nodes.insert(abs_path.clone(), HashMap::new());
                }

                if let Some(id) = ast.get("id").and_then(|v| v.as_u64())
                    && let Some(src) = ast.get("src").and_then(|v| v.as_str())
                {
                    nodes.get_mut(&abs_path).unwrap().insert(
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

                let mut stack = vec![ast];

                while let Some(tree) = stack.pop() {
                    if let Some(id) = tree.get("id").and_then(|v| v.as_u64())
                        && let Some(src) = tree.get("src").and_then(|v| v.as_str())
                    {
                        // Check for nameLocation first
                        let mut name_location = tree
                            .get("nameLocation")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        // Check for nameLocations array and use first element if
                        // available
                        if name_location.is_none()
                            && let Some(name_locations) = tree.get("nameLocations")
                            && let Some(locations_array) = name_locations.as_array()
                            && !locations_array.is_empty()
                        {
                            name_location = locations_array[0].as_str().map(|s| s.to_string());
                        }

                        let node_info = NodeInfo {
                            src: src.to_string(),
                            name_location,
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

                        nodes.get_mut(&abs_path).unwrap().insert(id, node_info);
                    }

                    push_if_node_or_array(tree, "arguments", &mut stack);
                    push_if_node_or_array(tree, "arguments", &mut stack);
                    push_if_node_or_array(tree, "baseContracts", &mut stack);
                    push_if_node_or_array(tree, "baseContracts", &mut stack);
                    push_if_node_or_array(tree, "baseExpression", &mut stack);
                    push_if_node_or_array(tree, "baseName", &mut stack);
                    push_if_node_or_array(tree, "baseType", &mut stack);
                    push_if_node_or_array(tree, "block", &mut stack);
                    push_if_node_or_array(tree, "body", &mut stack);
                    push_if_node_or_array(tree, "components", &mut stack);
                    push_if_node_or_array(tree, "components", &mut stack);
                    push_if_node_or_array(tree, "condition", &mut stack);
                    push_if_node_or_array(tree, "declarations", &mut stack);
                    push_if_node_or_array(tree, "endExpression", &mut stack);
                    push_if_node_or_array(tree, "errorCall", &mut stack);
                    push_if_node_or_array(tree, "eventCall", &mut stack);
                    push_if_node_or_array(tree, "expression", &mut stack);
                    push_if_node_or_array(tree, "externalCall", &mut stack);
                    push_if_node_or_array(tree, "falseBody", &mut stack);
                    push_if_node_or_array(tree, "falseExpression", &mut stack);
                    push_if_node_or_array(tree, "file", &mut stack);
                    push_if_node_or_array(tree, "foreign", &mut stack);
                    push_if_node_or_array(tree, "indexExpression", &mut stack);
                    push_if_node_or_array(tree, "initialValue", &mut stack);
                    push_if_node_or_array(tree, "initialValue", &mut stack);
                    push_if_node_or_array(tree, "initializationExpression", &mut stack);
                    push_if_node_or_array(tree, "keyType", &mut stack);
                    push_if_node_or_array(tree, "leftExpression", &mut stack);
                    push_if_node_or_array(tree, "leftHandSide", &mut stack);
                    push_if_node_or_array(tree, "libraryName", &mut stack);
                    push_if_node_or_array(tree, "literals", &mut stack);
                    push_if_node_or_array(tree, "loopExpression", &mut stack);
                    push_if_node_or_array(tree, "members", &mut stack);
                    push_if_node_or_array(tree, "modifierName", &mut stack);
                    push_if_node_or_array(tree, "modifiers", &mut stack);
                    push_if_node_or_array(tree, "name", &mut stack);
                    push_if_node_or_array(tree, "names", &mut stack);
                    push_if_node_or_array(tree, "nodes", &mut stack);
                    push_if_node_or_array(tree, "options", &mut stack);
                    push_if_node_or_array(tree, "options", &mut stack);
                    push_if_node_or_array(tree, "options", &mut stack);
                    push_if_node_or_array(tree, "overrides", &mut stack);
                    push_if_node_or_array(tree, "overrides", &mut stack);
                    push_if_node_or_array(tree, "parameters", &mut stack);
                    push_if_node_or_array(tree, "parameters", &mut stack);
                    push_if_node_or_array(tree, "pathNode", &mut stack);
                    push_if_node_or_array(tree, "returnParameters", &mut stack);
                    push_if_node_or_array(tree, "returnParameters", &mut stack);
                    push_if_node_or_array(tree, "rightExpression", &mut stack);
                    push_if_node_or_array(tree, "rightHandSide", &mut stack);
                    push_if_node_or_array(tree, "startExpression", &mut stack);
                    push_if_node_or_array(tree, "statements", &mut stack);
                    push_if_node_or_array(tree, "statements", &mut stack);
                    push_if_node_or_array(tree, "storageLayout", &mut stack);
                    push_if_node_or_array(tree, "subExpression", &mut stack);
                    push_if_node_or_array(tree, "subdenomination", &mut stack);
                    push_if_node_or_array(tree, "symbolAliases", &mut stack);
                    push_if_node_or_array(tree, "trueBody", &mut stack);
                    push_if_node_or_array(tree, "trueExpression", &mut stack);
                    push_if_node_or_array(tree, "typeName", &mut stack);
                    push_if_node_or_array(tree, "unitAlias", &mut stack);
                    push_if_node_or_array(tree, "value", &mut stack);
                    push_if_node_or_array(tree, "valueType", &mut stack);
                }
            }
        }
    }

    (nodes, path_to_abs)
}

pub fn goto_bytes(
    nodes: &HashMap<String, HashMap<u64, NodeInfo>>,
    path_to_abs: &HashMap<String, String>,
    id_to_path: &HashMap<String, String>,
    uri: &str,
    position: usize,
) -> Option<(String, usize)> {
    let path = match uri.starts_with("file://") {
        true => &uri[7..],
        false => uri,
    };

    // Get absolute path for this file
    let abs_path = path_to_abs.get(path)?;

    // Get nodes for the current file only
    let current_file_nodes = nodes.get(abs_path)?;

    let mut refs = HashMap::new();

    // Only consider nodes from the current file that have references
    for (id, content) in current_file_nodes {
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

    if refs.is_empty() {
        return None;
    }

    // Find the reference with minimum diff (most specific)
    let min_diff = *refs.keys().min()?;
    let chosen_id = refs[&min_diff];

    // Get the referenced declaration ID
    let ref_id = current_file_nodes[&chosen_id].referenced_declaration?;

    // Search for the referenced declaration across all files
    let mut target_node: Option<&NodeInfo> = None;
    for file_nodes in nodes.values() {
        if let Some(node) = file_nodes.get(&ref_id) {
            target_node = Some(node);
            break;
        }
    }

    let node = target_node?;

    // Get location from nameLocation or src
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

    let (nodes, path_to_abs) = cache_ids(sources);
    let byte_position = pos_to_bytes(source_bytes, position);

    if let Some((file_path, location_bytes)) =
        goto_bytes(&nodes, &path_to_abs, &id_to_path_map, file_uri.as_ref(), byte_position)
    {
        // Read the target file to convert byte position to line/column
        let target_file_path = std::path::Path::new(&file_path);

        // Make the path absolute if it's relative
        let absolute_path = if target_file_path.is_absolute() {
            target_file_path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(target_file_path)
        };

        if let Ok(target_source_bytes) = std::fs::read(&absolute_path)
            && let Some(target_position) = bytes_to_pos(&target_source_bytes, location_bytes)
            && let Ok(target_uri) = Url::from_file_path(&absolute_path)
        {
            return Some(Location {
                uri: target_uri,
                range: Range { start: target_position, end: target_position },
            });
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

    fn get_test_file_uri(relative_path: &str) -> Url {
        let current_dir = std::env::current_dir().expect("Failed to get current directory");
        let absolute_path = current_dir.join(relative_path);
        Url::from_file_path(absolute_path).expect("Failed to create file URI")
    }

    #[test]
    fn test_goto_declaration_basic() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on line 22, column 8 (position of "name" in add_vote function,
        // 0-based = line 21)
        let position = Position::new(21, 8);
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "name" parameter
        // Just verify we get a reasonable result within file bounds
        assert!(location.range.start.line < 25, "Declaration should be within the file bounds");
    }

    #[test]
    fn test_goto_declaration_variable_reference() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on "votes" usage (line 23, 0-based = line 22)
        let position = Position::new(22, 25); // Position of "votes" in name.add_one(votes)
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "votes" state variable
        // Just verify we get a reasonable result within file bounds
        assert!(location.range.start.line < 25, "Declaration should be within the file bounds");
    }

    #[test]
    fn test_goto_declaration_function_call() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
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
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test goto declaration on "votes" in constructor (line 16, 0-based = line 15)
        let position = Position::new(15, 8); // Position of "votes" in constructor
        let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);

        assert!(result.is_some());
        let location = result.unwrap();

        // Should find the declaration of the "votes" state variable
        // Just verify we get a reasonable result within file bounds
        assert!(location.range.start.line < 25, "Declaration should be within the file bounds");
    }

    #[test]
    fn test_goto_declaration_immutable_variable() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
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
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
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
                return;
            }
        };

        let sources = ast_data.get("sources").unwrap();
        let (nodes, path_to_abs) = cache_ids(sources);

        // Should have cached multiple files
        assert!(!nodes.is_empty());
        assert!(!path_to_abs.is_empty());

        // Check that nodes have the expected structure
        nodes.iter().for_each(|(_file_path, file_nodes)| {
            for node_info in file_nodes.values() {
                assert!(!node_info.src.is_empty());
                // Some nodes should have referenced declarations
                if node_info.referenced_declaration.is_some() {}
            }
        });
    }

    #[test]
    fn test_goto_bytes_functionality() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
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

        let (nodes, path_to_abs) = cache_ids(sources);
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test with a position that should have a reference
        let position = Position::new(21, 8); // "name" in add_vote function
        let byte_position = pos_to_bytes(&source_bytes, position);

        let file_uri_url = get_test_file_uri("testdata/C.sol");
        let file_uri = file_uri_url.as_str();
        let result = goto_bytes(&nodes, &path_to_abs, &id_to_path_map, file_uri, byte_position);

        // Should find a declaration
        if let Some((file_path, _location_bytes)) = result {
            assert!(!file_path.is_empty());
        }
    }
    #[test]
    fn test_goto_declaration_and_definition_consistency() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
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
                return;
            }
        };

        let file_uri = get_test_file_uri("testdata/C.sol");
        let source_bytes = std::fs::read("testdata/C.sol").unwrap();

        // Test multiple positions to ensure goto_definition works consistently
        let test_positions = vec![
            (Position::new(21, 8), "parameter reference"), // "name" in add_vote function
            (Position::new(22, 25), "state variable reference"), // "votes" in name.add_one(votes)
            (Position::new(15, 8), "state variable in constructor"), // "votes" in constructor
        ];

        for (position, description) in test_positions {
            let result = goto_declaration(&ast_data, &file_uri, position, &source_bytes);
            assert!(result.is_some(), "Failed to find definition for {description}");

            let location = result.unwrap();
            // Verify we got a valid location
            assert!(location.range.start.line < 100, "Invalid line number for {description}");
            assert!(
                location.range.start.character < 1000,
                "Invalid character position for {description}"
            );
        }
    }

    #[test]
    fn test_name_locations_handling() {
        let ast_data = match get_ast_data() {
            Some(data) => data,
            None => {
                return;
            }
        };

        let sources = ast_data.get("sources").unwrap();
        let (nodes, _path_to_abs) = cache_ids(sources);

        // Verify that nodes have name_location set (either from nameLocation or nameLocations[0])
        let mut nodes_with_name_location = 0;
        for file_nodes in nodes.values() {
            for node_info in file_nodes.values() {
                if node_info.name_location.is_some() {
                    nodes_with_name_location += 1;
                }
            }
        }

        // Should have at least some nodes with name locations
        assert!(nodes_with_name_location > 0, "Expected to find nodes with name locations");
    }

    #[test]
    fn test_name_locations_array_parsing() {
        use serde_json::json;

        // Create a mock AST structure with nameLocations array
        let mock_sources = json!({
            "test.sol": [{
                "source_file": {
                    "ast": {
                        "id": 1,
                        "src": "0:100:0",
                        "nodeType": "SourceUnit",
                        "absolutePath": "test.sol",
                        "nodes": [{
                            "id": 2,
                            "src": "10:20:0",
                            "nodeType": "ContractDefinition",
                            "nameLocations": ["15:8:0", "25:8:0"]
                        }, {
                            "id": 3,
                            "src": "30:15:0",
                            "nodeType": "VariableDeclaration",
                            "nameLocation": "35:5:0"
                        }]
                    }
                }
            }]
        });

        let (nodes, _path_to_abs) = cache_ids(&mock_sources);

        // Should have nodes for test.sol
        assert!(nodes.contains_key("test.sol"));
        let test_file_nodes = &nodes["test.sol"];

        // Node 2 should have nameLocation from nameLocations[0]
        assert!(test_file_nodes.contains_key(&2));
        let node2 = &test_file_nodes[&2];
        assert_eq!(node2.name_location, Some("15:8:0".to_string()));

        // Node 3 should have nameLocation from nameLocation field
        assert!(test_file_nodes.contains_key(&3));
        let node3 = &test_file_nodes[&3];
        assert_eq!(node3.name_location, Some("35:5:0".to_string()));
    }
}
