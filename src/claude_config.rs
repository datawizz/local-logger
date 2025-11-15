//! Claude Code configuration management
//!
//! This module provides functionality to install and uninstall local-logger
//! from Claude Code's configuration files:
//! - ~/.claude.json (MCP servers)
//! - ~/.claude/settings.json (hooks)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// MCP server configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub command: String,
    pub args: Vec<String>,
}

/// Claude .claude.json configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeConfig {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mcp_servers: HashMap<String, McpServer>,

    /// Preserve all other fields in the JSON
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Hook command configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookCommand {
    #[serde(rename = "type")]
    pub command_type: String,
    pub command: String,
}

/// Hook entry containing multiple hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookCommand>,
}

/// Claude ~/.claude/settings.json configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeSettings {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub hooks: HashMap<String, Vec<HookEntry>>,

    /// Preserve all other fields in the JSON
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Get the path to ~/.claude.json
fn get_claude_config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir())
        .context("Could not determine home directory")?;
    Ok(home.join(".claude.json"))
}

/// Get the path to ~/.claude/settings.json
fn get_claude_settings_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir())
        .context("Could not determine home directory")?;
    Ok(home.join(".claude").join("settings.json"))
}

/// Read and parse a JSON file, or return default if it doesn't exist
fn read_json_file<T: Default + for<'de> Deserialize<'de>>(path: &PathBuf) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    if content.trim().is_empty() {
        return Ok(T::default());
    }

    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON in {}", path.display()))
}

/// Write JSON to a file atomically (write to temp file, then rename)
fn write_json_file<T: Serialize>(path: &PathBuf, data: &T) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    // Write to temporary file using a suffix to avoid conflicts with dotfiles
    let temp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)
        .context("Failed to serialize JSON")?;

    let mut file = fs::File::create(&temp_path)
        .with_context(|| format!("Failed to create temporary file {}", temp_path.display()))?;

    file.write_all(json.as_bytes())
        .context("Failed to write JSON data")?;

    file.write_all(b"\n")
        .context("Failed to write trailing newline")?;

    file.sync_all()
        .context("Failed to sync file to disk")?;

    drop(file);

    // Atomic rename
    fs::rename(&temp_path, path)
        .with_context(|| format!("Failed to rename {} to {}", temp_path.display(), path.display()))?;

    Ok(())
}

/// Install local-logger into Claude Code configuration
pub fn install_claude_config(quiet: bool) -> Result<()> {
    if !quiet {
        println!("Installing local-logger into Claude Code configuration...");
    }

    // Install MCP server entry
    let config_path = get_claude_config_path()?;
    let mut config: ClaudeConfig = read_json_file(&config_path)?;

    config.mcp_servers.insert(
        "local-logger".to_string(),
        McpServer {
            command: "local-logger".to_string(),
            args: vec!["serve".to_string()],
        },
    );

    write_json_file(&config_path, &config)?;

    if !quiet {
        println!("  ✓ Added MCP server to {}", config_path.display());
    }

    // Install hooks
    let settings_path = get_claude_settings_path()?;
    let mut settings: ClaudeSettings = read_json_file(&settings_path)?;

    let hook_types = vec![
        "PreToolUse",
        "PostToolUse",
        "UserPromptSubmit",
        "Stop",
        "SubagentStop",
        "PreCompact",
        "Notification",
    ];

    let local_logger_hook = HookCommand {
        command_type: "command".to_string(),
        command: "local-logger hook".to_string(),
    };

    for hook_type in hook_types {
        let entries = settings.hooks.entry(hook_type.to_string()).or_insert_with(Vec::new);

        // Check if local-logger hook already exists in this hook type
        let has_local_logger = entries.iter().any(|entry| {
            entry.hooks.iter().any(|h| h.command == "local-logger hook")
        });

        if !has_local_logger {
            // Add a new hook entry for local-logger
            entries.push(HookEntry {
                matcher: if hook_type == "UserPromptSubmit"
                    || hook_type == "Stop"
                    || hook_type == "SubagentStop"
                    || hook_type == "PreCompact"
                    || hook_type == "Notification" {
                    None
                } else {
                    Some("".to_string())
                },
                hooks: vec![local_logger_hook.clone()],
            });
        }
    }

    write_json_file(&settings_path, &settings)?;

    if !quiet {
        println!("  ✓ Added hooks to {}", settings_path.display());
        println!("\n✓ Installation complete!");
    }

    Ok(())
}

/// Uninstall local-logger from Claude Code configuration
///
/// This function surgically removes ONLY local-logger entries while preserving
/// all other configuration. It will:
/// - Remove the local-logger MCP server entry
/// - Remove only local-logger hooks from each hook type
/// - Preserve all other hooks
/// - Clean up empty hook type arrays
/// - Remove the hooks object if it becomes empty
pub fn uninstall_claude_config(quiet: bool) -> Result<()> {
    if !quiet {
        println!("Removing local-logger from Claude Code configuration...");
    }

    let mut changes_made = false;

    // Remove MCP server entry
    let config_path = get_claude_config_path()?;
    if config_path.exists() {
        let mut config: ClaudeConfig = read_json_file(&config_path)?;

        if config.mcp_servers.remove("local-logger").is_some() {
            write_json_file(&config_path, &config)?;
            if !quiet {
                println!("  ✓ Removed MCP server from {}", config_path.display());
            }
            changes_made = true;
        } else if !quiet {
            println!("  · No local-logger MCP server found in {}", config_path.display());
        }
    } else if !quiet {
        println!("  · File not found: {}", config_path.display());
    }

    // Remove hooks surgically
    let settings_path = get_claude_settings_path()?;
    if settings_path.exists() {
        let mut settings: ClaudeSettings = read_json_file(&settings_path)?;
        let mut hooks_removed = 0;
        let mut hooks_preserved = 0;

        // Process each hook type
        for (_hook_type, entries) in settings.hooks.iter_mut() {
            // Filter each hook entry
            let mut filtered_entries = Vec::new();

            for entry in entries.iter() {
                // Filter hooks within this entry
                let filtered_hooks: Vec<HookCommand> = entry
                    .hooks
                    .iter()
                    .filter(|h| {
                        if h.command == "local-logger hook" {
                            hooks_removed += 1;
                            false
                        } else {
                            hooks_preserved += 1;
                            true
                        }
                    })
                    .cloned()
                    .collect();

                // Only keep entry if it has remaining hooks
                if !filtered_hooks.is_empty() {
                    filtered_entries.push(HookEntry {
                        matcher: entry.matcher.clone(),
                        hooks: filtered_hooks,
                    });
                }
            }

            *entries = filtered_entries;
        }

        // Remove empty hook types
        settings.hooks.retain(|_hook_type, entries| !entries.is_empty());

        if hooks_removed > 0 {
            write_json_file(&settings_path, &settings)?;
            if !quiet {
                if hooks_preserved > 0 {
                    println!(
                        "  ✓ Removed {} local-logger hook(s) from {} (preserved {} other hook(s))",
                        hooks_removed,
                        settings_path.display(),
                        hooks_preserved
                    );
                } else {
                    println!(
                        "  ✓ Removed {} local-logger hook(s) from {}",
                        hooks_removed,
                        settings_path.display()
                    );
                }
            }
            changes_made = true;
        } else if !quiet {
            println!("  · No local-logger hooks found in {}", settings_path.display());
        }
    } else if !quiet {
        println!("  · File not found: {}", settings_path.display());
    }

    if !quiet {
        if changes_made {
            println!("\n✓ Uninstallation complete!");
        } else {
            println!("\n· No local-logger configuration found");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_home(tmp_dir: &TempDir) {
        let home = tmp_dir.path();
        std::env::set_var("HOME", home);
    }

    #[test]
    #[serial]
    fn test_install_creates_files() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();

        let config_path = get_claude_config_path().unwrap();
        let settings_path = get_claude_settings_path().unwrap();

        assert!(config_path.exists());
        assert!(settings_path.exists());
    }

    #[test]
    #[serial]
    fn test_install_adds_mcp_server() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();

        let config_path = get_claude_config_path().unwrap();
        let config: ClaudeConfig = read_json_file(&config_path).unwrap();

        assert!(config.mcp_servers.contains_key("local-logger"));
        let server = &config.mcp_servers["local-logger"];
        assert_eq!(server.command, "local-logger");
        assert_eq!(server.args, vec!["serve"]);
    }

    #[test]
    #[serial]
    fn test_install_adds_hooks() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();

        let settings_path = get_claude_settings_path().unwrap();
        let settings: ClaudeSettings = read_json_file(&settings_path).unwrap();

        let expected_hooks = vec![
            "PreToolUse",
            "PostToolUse",
            "UserPromptSubmit",
            "Stop",
            "SubagentStop",
            "PreCompact",
            "Notification",
        ];

        for hook_type in expected_hooks {
            assert!(settings.hooks.contains_key(hook_type));
            let entries = &settings.hooks[hook_type];
            assert!(!entries.is_empty());

            let has_local_logger = entries.iter().any(|entry| {
                entry.hooks.iter().any(|h| h.command == "local-logger hook")
            });
            assert!(has_local_logger);
        }
    }

    #[test]
    #[serial]
    fn test_install_is_idempotent() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();
        install_claude_config(true).unwrap();

        let settings_path = get_claude_settings_path().unwrap();
        let settings: ClaudeSettings = read_json_file(&settings_path).unwrap();

        // Should only have one local-logger hook per type
        for entries in settings.hooks.values() {
            let local_logger_count = entries
                .iter()
                .flat_map(|entry| &entry.hooks)
                .filter(|h| h.command == "local-logger hook")
                .count();
            assert_eq!(local_logger_count, 1);
        }
    }

    #[test]
    #[serial]
    fn test_uninstall_removes_mcp_server() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();
        uninstall_claude_config(true).unwrap();

        let config_path = get_claude_config_path().unwrap();
        let config: ClaudeConfig = read_json_file(&config_path).unwrap();

        assert!(!config.mcp_servers.contains_key("local-logger"));
    }

    #[test]
    #[serial]
    fn test_uninstall_removes_only_local_logger_hooks() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();

        // Add a custom hook
        let settings_path = get_claude_settings_path().unwrap();
        let mut settings: ClaudeSettings = read_json_file(&settings_path).unwrap();

        settings.hooks.get_mut("PreToolUse").unwrap().push(HookEntry {
            matcher: Some("".to_string()),
            hooks: vec![HookCommand {
                command_type: "command".to_string(),
                command: "custom-hook".to_string(),
            }],
        });

        write_json_file(&settings_path, &settings).unwrap();

        // Uninstall
        uninstall_claude_config(true).unwrap();

        // Check that custom hook is preserved
        let settings: ClaudeSettings = read_json_file(&settings_path).unwrap();
        assert!(settings.hooks.contains_key("PreToolUse"));

        let entries = &settings.hooks["PreToolUse"];
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hooks[0].command, "custom-hook");

        // Check that local-logger hooks are gone from all types
        for entries in settings.hooks.values() {
            let has_local_logger = entries.iter().any(|entry| {
                entry.hooks.iter().any(|h| h.command == "local-logger hook")
            });
            assert!(!has_local_logger);
        }
    }

    #[test]
    #[serial]
    fn test_uninstall_is_idempotent() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        install_claude_config(true).unwrap();
        uninstall_claude_config(true).unwrap();
        uninstall_claude_config(true).unwrap(); // Run twice

        let config_path = get_claude_config_path().unwrap();
        let config: ClaudeConfig = read_json_file(&config_path).unwrap();

        assert!(!config.mcp_servers.contains_key("local-logger"));
    }

    #[test]
    #[serial]
    fn test_preserves_other_config_fields() {
        let tmp_dir = TempDir::new().unwrap();
        setup_test_home(&tmp_dir);

        // Create initial config with custom fields
        let config_path = get_claude_config_path().unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            r#"{"mcpServers":{},"customField":"customValue","numStartups":42}"#,
        )
        .unwrap();

        install_claude_config(true).unwrap();

        let config: ClaudeConfig = read_json_file(&config_path).unwrap();
        assert_eq!(
            config.other.get("customField"),
            Some(&serde_json::Value::String("customValue".to_string()))
        );
        assert_eq!(
            config.other.get("numStartups"),
            Some(&serde_json::Value::Number(42.into()))
        );
    }
}
