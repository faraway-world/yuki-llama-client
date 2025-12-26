use std::io::{self, Write};
use std::fs;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

const SERVER_URL: &str = "http://127.0.0.1:8080/v1/chat/completions";
const HISTORY_FILE: &str = "history.json";

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a Vec<Message>,
    stream: bool,
}

#[derive(Deserialize)]
struct StreamChunk { choices: Vec<Choice> }
#[derive(Deserialize)]
struct Choice { delta: Delta }
#[derive(Deserialize)]
struct Delta { content: Option<String> }

fn load_history() -> Vec<Message> {
    if let Ok(data) = fs::read_to_string(HISTORY_FILE) {
        if let Ok(msgs) = serde_json::from_str::<Vec<Message>>(&data) {
            println!("\x1b[93m--- Loaded history ({} messages) ---\x1b[0m", msgs.len());
            return msgs;
        }
    }
    Vec::new()
}

fn save_history(messages: &Vec<Message>) {
    if let Ok(json) = serde_json::to_string_pretty(messages) {
        let _ = fs::write(HISTORY_FILE, json);
    }
}

async fn chat(client: &Client, messages: &Vec<Message>) -> anyhow::Result<String> {
    let payload = ChatRequest { model: "local", messages, stream: true };
    let response = client.post(SERVER_URL).json(&payload).send().await?;
    let mut stream = response.bytes_stream();
    let mut full_reply = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let text = String::from_utf8_lossy(&chunk);
        for line in text.lines() {
            if !line.starts_with("data: ") { continue; }
            let data = &line[6..];
            if data == "[DONE]" { return Ok(full_reply); }
            if let Ok(obj) = serde_json::from_str::<StreamChunk>(data) {
                if let Some(choice) = obj.choices.first() {
                    if let Some(content) = &choice.delta.content {
                        print!("{}", content);
                        io::stdout().flush()?;
                        full_reply.push_str(content);
                    }
                }
            }
        }
    }
    Ok(full_reply)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new();
    let mut messages = load_history();
    let mut rl = DefaultEditor::new()?;
    
    println!("\x1b[96mYuki Chat (Rust). Type 'exit' to quit or 'clear' to reset.\x1b[0m");

    loop {
        let readline = rl.readline("\nYou: ");
        match readline {
            Ok(line) => {
                let user = line.trim();
                if user.is_empty() { continue; }
                if user == "exit" || user == "quit" { break; }
                
                if user == "clear" {
                    messages.clear();
                    let _ = fs::remove_file(HISTORY_FILE);
                    println!("History cleared.");
                    continue;
                }

                let _ = rl.add_history_entry(user);
                messages.push(Message { role: "user".to_string(), content: user.to_string() });

                print!("\nAssistant: ");
                io::stdout().flush()?;

                match chat(&client, &messages).await {
                    Ok(reply) => {
                        messages.push(Message { role: "assistant".to_string(), content: reply });
                        save_history(&messages);
                    }
                    Err(e) => println!("\nError: {}", e),
                }
                println!();
            },
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(err) => { println!("Error: {:?}", err); break; }
        }
    }
    Ok(())
}