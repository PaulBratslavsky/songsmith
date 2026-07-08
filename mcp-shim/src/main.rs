//! MCP stdio server for Songsmith Studio.
//!
//! Claude Code / Claude Desktop spawn this binary (`claude mcp add songsmith
//! /path/to/mcp-shim`). It links the `song_core` crate directly and opens the same
//! libSQL database file the desktop app uses, then exposes the shared tool
//! registry over MCP's JSON-RPC stdio transport — so Claude drives exactly the
//! same tools the UI and the in-app agent loop do.
//!
//! Concurrency: every `tools/call` is dispatched on its own task, so a long
//! `run_stage` never blocks `ping`/`tools/list`/other calls (responses may
//! return out of order — valid JSON-RPC, matched by id). All stdout writes go
//! through ONE writer task fed by a channel, so response lines never interleave.
//!
//! The DB path comes from `SONGSMITH_DB` (the app writes this into the MCP
//! config it generates), falling back to the default app-data location.

use serde_json::{json, Value};
use song_core::{db, tools};
use tokio::io::AsyncBufReadExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_path = std::env::var("SONGSMITH_DB").unwrap_or_else(|_| default_db_path());
    let database = db::open(std::path::Path::new(&db_path)).await?;
    // db::connect sets PRAGMA busy_timeout so writes racing the desktop app wait
    // for the lock instead of surfacing "database is locked" as raw tool errors
    let conn = db::connect(&database).await?;

    // The single stdout writer: every response is one full JSON line, written
    // and flushed by this task alone — concurrent tool tasks can't interleave.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let writer = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        while let Some(resp) = rx.blocking_recv() {
            let Ok(line) = serde_json::to_string(&resp) else { continue };
            if writeln!(out, "{line}").is_err() {
                break; // client hung up
            }
            let _ = out.flush();
        }
    });

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        // Notifications (no id) get no response.
        match method {
            "initialize" => {
                let _ = tx.send(reply(id, json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "songsmith-studio", "version": "0.1.0" }
                })));
            }
            "tools/list" => {
                let list: Vec<Value> = tools::registry().iter().map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                })).collect();
                let _ = tx.send(reply(id, json!({ "tools": list })));
            }
            // Each call runs on its own task so a long run_stage can't block
            // the loop; the response channel serializes the actual writes.
            "tools/call" => {
                let conn = conn.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let args = params.get("arguments").cloned().unwrap_or(json!({}));
                    let settings = db::get_settings(&conn).await.unwrap_or_default();
                    let resp = match tools::dispatch(&conn, &settings, &name, &args).await {
                        Ok(result) => reply(id, json!({
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }]
                        })),
                        Err(e) => error(id, -32000, &e.to_string()),
                    };
                    let _ = tx.send(resp);
                });
            }
            "ping" => {
                let _ = tx.send(reply(id, json!({})));
            }
            _ if id.is_some() => {
                let _ = tx.send(error(id, -32601, "method not found"));
            }
            _ => {}
        }
    }

    // stdin closed: drop our sender; in-flight tool tasks keep their clones, so
    // the writer drains every remaining response before exiting.
    drop(tx);
    let _ = writer.await;
    Ok(())
}

fn reply(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
fn error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn default_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    format!("{home}/Library/Application Support/com.songsmithstudio.desktop/songsmith-studio.db")
}
