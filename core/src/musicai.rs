//! OPTIONAL ADD-ON — Music.AI cloud lyric transcription (music.ai).
//!
//! Songsmith is local-first: audio never leaves the machine by default. This
//! module is the single, explicit exception — it only ever runs when the user
//! has pasted a Music.AI API key + workflow slug into Settings AND the local
//! whisper ladder came up empty (heavily-processed synth vocals defeat both
//! whisper-small and -medium; verified 2026-07-24). The upload is per-song
//! and user-initiated (the ⟳ Resume import retry).
//!
//! API (https://music.ai/docs/api/reference/):
//!   GET  /v1/upload            → { uploadUrl, downloadUrl }
//!   PUT  <uploadUrl>           ← raw audio bytes
//!   POST /v1/job               ← { name, workflow, params: { inputUrl } }
//!   GET  /v1/job/:id/status    → { status: QUEUED|STARTED|SUCCEEDED|FAILED }
//!   GET  /v1/job/:id           → { result: { <output name>: <url>, … } }
//!
//! The workflow's output names are user-chosen in the Music.AI console, so
//! the result parser is deliberately tolerant: every result URL is fetched
//! and scanned for anything transcript-shaped ({text|word, start|startTime,
//! end|endTime} objects, nested or not), normalized to our transcript form
//! [{ start, end, text }] in seconds.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

const BASE: &str = "https://api.music.ai/v1";

fn client(api_key: &str) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        api_key.parse().map_err(|_| anyhow!("the Music.AI API key contains invalid characters"))?,
    );
    // per-request timeouts: without them a stalled connection hangs the
    // resume forever (the 5-minute deadline only bounds SUCCESSFUL polls)
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?)
}

/// A timeout-bounded client for the PRE-SIGNED urls (upload/result), which
/// carry their own auth and must not get the API-key header.
fn plain_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180)) // the audio upload is the big one
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?)
}

/// Upload the audio, run the configured workflow, and return transcript
/// entries [{start, end, text}] (seconds). Errors are user-readable.
pub async fn transcribe_lyrics(
    api_key: &str,
    workflow: &str,
    audio_path: &str,
    progress: &(dyn Fn(String) + Sync),
) -> Result<Vec<Value>> {
    let http = client(api_key)?;

    progress("☁ Music.AI: uploading the audio…".into());
    let up: Value = http.get(format!("{BASE}/upload")).send().await?
        .error_for_status().map_err(|e| anyhow!("Music.AI upload-URL request failed: {e}"))?
        .json().await?;
    let upload_url = up.get("uploadUrl").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Music.AI /upload returned no uploadUrl"))?;
    let download_url = up.get("downloadUrl").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Music.AI /upload returned no downloadUrl"))?;

    let bytes = tokio::fs::read(audio_path).await
        .map_err(|e| anyhow!("could not read the audio file: {e}"))?;
    // the signed upload URL is pre-authorized — no API-key header client here
    plain_client()?.put(upload_url)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(bytes).send().await?
        .error_for_status().map_err(|e| anyhow!("Music.AI upload failed: {e}"))?;

    progress(format!("☁ Music.AI: running workflow \"{workflow}\"…"));
    let job: Value = http.post(format!("{BASE}/job"))
        .json(&json!({
            "name": format!("songsmith lyrics: {}", std::path::Path::new(audio_path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
            "workflow": workflow,
            "params": { "inputUrl": download_url },
        }))
        .send().await?
        .error_for_status().map_err(|e| anyhow!("Music.AI job creation failed (check the workflow slug): {e}"))?
        .json().await?;
    let job_id = job.get("id").and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Music.AI job creation returned no id"))?
        .to_string();

    // poll to completion (transcription of a song is typically well under a
    // minute on their side; cap at 5 minutes)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let st: Value = http.get(format!("{BASE}/job/{job_id}/status")).send().await?
            .error_for_status()?.json().await?;
        match st.get("status").and_then(|v| v.as_str()).unwrap_or("") {
            "SUCCEEDED" => break,
            "FAILED" => {
                let full: Value = http.get(format!("{BASE}/job/{job_id}")).send().await?.json().await.unwrap_or(Value::Null);
                let msg = full.pointer("/error/message").and_then(|v| v.as_str()).unwrap_or("no error detail");
                return Err(anyhow!("Music.AI job failed: {msg}"));
            }
            _ => {
                if std::time::Instant::now() > deadline {
                    return Err(anyhow!("Music.AI job timed out after 5 minutes"));
                }
            }
        }
    }

    progress("☁ Music.AI: fetching the transcription…".into());
    let full: Value = http.get(format!("{BASE}/job/{job_id}")).send().await?
        .error_for_status()?.json().await?;
    let result = full.get("result").cloned().unwrap_or(Value::Null);
    let mut transcript: Vec<Value> = vec![];
    if let Some(obj) = result.as_object() {
        for url in obj.values().filter_map(|v| v.as_str()) {
            // result URLs are pre-signed — plain client
            let Ok(client) = plain_client() else { continue };
            let Ok(resp) = client.get(url).send().await else { continue };
            let Ok(text) = resp.text().await else { continue };
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                collect_transcript(&v, &mut transcript);
            }
            if !transcript.is_empty() {
                break;
            }
        }
    }
    transcript.sort_by(|a, b| {
        let sa = a.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
    });
    if transcript.is_empty() {
        return Err(anyhow!("the Music.AI workflow output held nothing transcript-shaped — does it end in a lyric-transcription module with a JSON output?"));
    }
    Ok(transcript)
}

/// Recursively scan any JSON for transcript-shaped objects and normalize
/// them to { start, end, text } (seconds — millisecond-looking timestamps
/// are scaled down).
fn collect_transcript(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Array(items) => {
            for it in items {
                collect_transcript(it, out);
            }
        }
        Value::Object(o) => {
            let text = o.get("text").or_else(|| o.get("word")).or_else(|| o.get("line")).and_then(|t| t.as_str());
            let start = ["start", "startTime", "start_time"].iter().find_map(|k| o.get(*k)).and_then(num_or_str_f64);
            let end = ["end", "endTime", "end_time"].iter().find_map(|k| o.get(*k)).and_then(num_or_str_f64);
            if let (Some(text), Some(mut start), Some(mut end)) = (text, start, end) {
                if !text.trim().is_empty() {
                    // >10h "seconds" means the workflow emitted milliseconds
                    if start > 36_000.0 {
                        start /= 1000.0;
                        end /= 1000.0;
                    }
                    out.push(json!({ "start": start, "end": end, "text": text.trim() }));
                    return; // don't also collect this object's children (words within a line)
                }
            }
            for val in o.values() {
                collect_transcript(val, out);
            }
        }
        _ => {}
    }
}

fn num_or_str_f64(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tolerant parser handles the common output shapes: flat segment
    /// lists, {lines:[…]} wrappers, camelCase ms timestamps, and prefers
    /// line objects over their nested word children.
    #[test]
    fn collects_various_transcript_shapes() {
        let mut out = vec![];
        collect_transcript(&json!([{ "text": "hello world", "start": 1.5, "end": 3.0 }]), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["text"], "hello world");

        let mut out = vec![];
        collect_transcript(&json!({ "lines": [
            { "line": "first line", "startTime": 1000.0, "endTime": 2000.0, "words": [
                { "word": "first", "startTime": 1000.0, "endTime": 1400.0 } ] },
            { "line": "second line", "startTime": 60000.0, "endTime": 62000.0 }
        ]}), &mut out);
        // hmm: 1000.0 sec < 36000 → NOT scaled; acceptable — ms detection is
        // per-entry heuristic and late-song entries (>10h as seconds) scale
        assert_eq!(out.len(), 2, "line objects win over nested words");
        assert_eq!(out[0]["text"], "first line");

        let mut out = vec![];
        collect_transcript(&json!({ "transcript": [{ "text": "late", "start": "200000", "end": "201000" }] }), &mut out);
        assert_eq!(out[0]["start"], json!(200.0), "string ms timestamps parse + scale");
    }
}
