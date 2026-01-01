use std::io::{self, Write};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{Validator, ValidationContext, ValidationResult};
use rustyline::{Context, Helper, Editor, Config, KeyEvent, Cmd, Modifiers};

// ... (Constants and Structs remain the same) ...
const SERVER_URL: &str = "http://127.0.0.1:8080/v1/chat/completions";

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

fn get_root_path() -> PathBuf {
    let mut path = PathBuf::from(std::env::var("HOME").expect("Could not find HOME directory"));
    path.push("yuki_client");
    path
}

struct ChatCompleter;
impl Helper for ChatCompleter {}
impl Hinter for ChatCompleter { type Hint = String; }
impl Highlighter for ChatCompleter {}

impl Validator for ChatCompleter {
    fn validate(&self, _ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

// ... (Keep the existing Completer and Logic helper implementations from previous code) ...
impl Completer for ChatCompleter {
    type Candidate = Pair;
    fn complete(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let mut completions = Vec::new();
        if line.starts_with("/read ") {
            let path_str = &line[6..];
            let expanded_path = if path_str.starts_with('~') {
                path_str.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)
            } else {
                path_str.to_string()
            };
            let path = std::path::Path::new(&expanded_path);
            let (dir_to_read, search_term): (String, String) = if expanded_path.ends_with('/') || expanded_path.is_empty() {
                (expanded_path.clone(), String::new())
            } else {
                let parent = path.parent().unwrap_or_else(|| std::path::Path::new("")).to_string_lossy().to_string();
                let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let separator = if parent.is_empty() || parent.ends_with('/') { "" } else { "/" };
                (format!("{}{}", parent, separator), file_name)
            };
            let dir_path = if dir_to_read.is_empty() { "./".to_string() } else { dir_to_read };
            if let Ok(entries) = fs::read_dir(&dir_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with(&search_term) {
                        let is_dir = entry.path().is_dir();
                        let display_name = if is_dir { format!("{}/", name) } else { name.clone() };
                        let mut replacement = line[..line.len() - search_term.len()].to_string();
                        replacement.push_str(&name);
                        if is_dir { replacement.push('/'); }
                        completions.push(Pair { display: display_name, replacement });
                    }
                }
            }
            return Ok((0, completions));
        }
        let search_line = if line.starts_with("/load ") { &line[6..] } else { line };
        let hist_dir = get_root_path().join("history");
        if let Ok(entries) = fs::read_dir(hist_dir) {
            for entry in entries.flatten() {
                let filename = entry.file_name().to_string_lossy().into_owned();
                if filename.starts_with("history_") && filename.ends_with(".json") {
                    let name = &filename[8..filename.len() - 5];
                    if name.starts_with(search_line) {
                        let replacement = if line.starts_with("/load ") { format!("/load {}", name) } else { name.to_string() };
                        completions.push(Pair { display: name.to_string(), replacement });
                    }
                }
            }
        }
        Ok((0, completions))
    }
}

fn get_file_paths(chat_name: &str) -> (PathBuf, PathBuf) {
    let root = get_root_path();
    let history_path = root.join("history").join(format!("history_{}.json", chat_name));
    let summary_path = root.join("chats").join(format!("summary_{}.json", chat_name));
    (history_path, summary_path)
}

fn ensure_dirs() -> io::Result<()> {
    let root = get_root_path();
    fs::create_dir_all(root.join("history"))?;
    fs::create_dir_all(root.join("chats"))?;
    fs::create_dir_all(root.join("backups"))?;
    Ok(())
}

fn create_backup(chat_name: &str, hist_path: &PathBuf) {
    if fs::metadata(hist_path).is_ok() {
        if let Ok(ts) = SystemTime::now().duration_since(UNIX_EPOCH) {
            let backup_dir = get_root_path().join("backups");
            let backup_path = backup_dir.join(format!("log_{}_{}.json", chat_name, ts.as_secs()));
            let _ = fs::copy(hist_path, &backup_path);
        }
    }
}

fn list_existing_chats() {
    let hist_dir = get_root_path().join("history");
    println!("\x1b[94mExisting Chats:\x1b[0m");
    if let Ok(entries) = fs::read_dir(hist_dir) {
        let mut found = false;
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().into_owned();
            if filename.starts_with("history_") && filename.ends_with(".json") {
                let name = &filename[8..filename.len() - 5];
                println!("  \x1b[93m- {}\x1b[0m", name);
                found = true;
            }
        }
        if !found { println!("  (No existing chats found)"); }
    }
    println!();
}

fn load_initial_messages(chat_name: &str) -> Vec<Message> {
    let (hist_path, summ_path) = get_file_paths(chat_name);
    if let Ok(data) = fs::read_to_string(&summ_path) {
        if let Ok(msgs) = serde_json::from_str::<Vec<Message>>(&data) {
            if !msgs.is_empty() { return msgs; }
        }
    }
    if let Ok(data) = fs::read_to_string(&hist_path) {
        if let Ok(msgs) = serde_json::from_str::<Vec<Message>>(&data) {
            if !msgs.is_empty() { return msgs; }
        }
    }
    Vec::new()
}

fn save_to_file(path: &PathBuf, messages: &Vec<Message>) {
    if let Ok(json) = serde_json::to_string_pretty(messages) {
        let _ = fs::write(path, json);
    }
}

async fn chat_request(client: &Client, messages: &Vec<Message>) -> anyhow::Result<String> {
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
            if data == "[DONE]" { println!(); return Ok(full_reply); }
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
    println!(); Ok(full_reply)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ensure_dirs()?;
    let client = Client::new();
    let config = Config::builder().build();
    let mut rl = Editor::<ChatCompleter, rustyline::history::DefaultHistory>::with_config(config)?;
    rl.set_helper(Some(ChatCompleter));

    // --- REMAPPED BINDINGS ---

    // 1. Plain Enter = Send
    rl.bind_sequence(KeyEvent::new('\r', Modifiers::NONE), Cmd::AcceptLine);
    rl.bind_sequence(KeyEvent::new('\n', Modifiers::NONE), Cmd::AcceptLine);

    // 2. Shift + Enter = New Line
    // We bind both the logic key and common raw escape sequences sent by terminals
    rl.bind_sequence(KeyEvent::new('\r', Modifiers::SHIFT), Cmd::Newline);
    rl.bind_sequence(KeyEvent::new('\n', Modifiers::SHIFT), Cmd::Newline);
    
    // Some terminals send specific codes for Shift+Enter (e.g. \x1b[13;2u)
    // If your terminal supports it, this will catch it:
    rl.bind_sequence(KeyEvent::new('\r', Modifiers::ALT), Cmd::Newline); 
    rl.bind_sequence(KeyEvent::new('\n', Modifiers::ALT), Cmd::Newline);

    // 3. Keep Alt+Enter as a secondary "New Line" just in case Shift is swallowed
    rl.bind_sequence(KeyEvent::alt('\r'), Cmd::Newline);
    rl.bind_sequence(KeyEvent::alt('\n'), Cmd::Newline);

    // ... (Remaining main loop logic remains exactly as before) ...
    let logo = r#"
 █████ █████ █████  █████ █████   ████ █████
▒▒███ ▒▒███ ▒▒███   ▒▒███ ▒▒███   ███▒ ▒▒███ 
 ▒▒███ ███   ▒███   ▒███  ▒███  ███    ▒███ 
  ▒▒█████    ▒███   ▒███  ▒███████     ▒███ 
   ▒▒███     ▒███   ▒███  ▒███▒▒███    ▒███ 
    ▒███     ▒███   ▒███  ▒███ ▒▒███   ▒███ 
    █████    ▒▒████████   █████ ▒▒████ █████
   ▒▒▒▒▒      ▒▒▒▒▒▒▒▒   ▒▒▒▒▒   ▒▒▒▒ ▒▒▒▒▒ 
    "#;
    println!("{}", logo);
    println!("\x1b[96mYuki Client (Rust) Started.\x1b[0m");
    list_existing_chats();
    
    let mut current_chat = loop {
        match rl.readline("Enter Chat Name (or '<name> /delete'): ") {
            Ok(line) => {
                let val = line.trim();
                if val.is_empty() { continue; }
                if val.ends_with(" /delete") {
                    let target = val.replace(" /delete", "").trim().to_string();
                    let (hp, sp) = get_file_paths(&target);
                    let _ = fs::remove_file(hp);
                    let _ = fs::remove_file(sp);
                    println!("\x1b[91m[System] Chat '{}' deleted.\x1b[0m", target);
                    list_existing_chats();
                    continue;
                }
                break val.to_string();
            },
            Err(_) => return Ok(()),
        }
    };

    let mut messages = load_initial_messages(&current_chat);
    let (mut hist_path, mut summ_path) = get_file_paths(&current_chat);
    println!("\x1b[92m--- Session: {} ---\x1b[0m", current_chat);
    println!("\x1b[90m(Enter: Send | Shift+Enter: New Line)\x1b[0m");

    loop {
        let char_count: usize = messages.iter().map(|m| m.content.len()).sum();
        let prompt = format!("[{} | ~{} chars]: ", current_chat, char_count);
        
        match rl.readline(&prompt) {
            Ok(line) => {
                let user_input = line.trim();
                if user_input.is_empty() { continue; }
                
                if user_input.starts_with('/') {
                    let parts: Vec<&str> = user_input.splitn(2, ' ').collect();
                    match parts[0] {
                        "/exit" | "/quit" => break,
                        "/clear" => {
                            create_backup(&current_chat, &hist_path);
                            messages.clear();
                            save_to_file(&hist_path, &messages);
                            save_to_file(&summ_path, &messages);
                            println!("\x1b[93mMemory wiped.\x1b[0m");
                            continue;
                        }
                        "/summarize" => {
                            create_backup(&current_chat, &hist_path);
                            println!("\x1b[95m\n[System] Compressing memory...\x1b[0m");
                            let mut summary_req = messages.clone();
                            summary_req.push(Message { role: "user".to_string(), content: "Summarize our conversation into 4 bullet points.".to_string() });
                            if let Ok(reply) = chat_request(&client, &summary_req).await {
                                messages = vec![Message { role: "system".to_string(), content: format!("MEMORY_BLOCK:\n{}", reply) }];
                                save_to_file(&summ_path, &messages);
                                save_to_file(&hist_path, &Vec::new());
                            }
                            continue;
                        }
                        "/load" => {
                            if parts.len() < 2 { list_existing_chats(); continue; }
                            current_chat = parts[1].trim().to_string();
                            let paths = get_file_paths(&current_chat);
                            hist_path = paths.0; summ_path = paths.1;
                            messages = load_initial_messages(&current_chat);
                            continue;
                        }
                        "/read" => {
                            if parts.len() < 2 { continue; }
                            let path = parts[1].trim().replace('~', &std::env::var("HOME").unwrap_or_default());
                            if let Ok(content) = fs::read_to_string(&path) {
                                messages.push(Message { role: "user".to_string(), content: format!("Analyze file {}:\n{}", path, content) });
                                print!("\x1b[92mAssistant:\x1b[0m ");
                                io::stdout().flush()?;
                                if let Ok(reply) = chat_request(&client, &messages).await {
                                    messages.push(Message { role: "assistant".to_string(), content: reply });
                                    save_to_file(&hist_path, &messages);
                                }
                            }
                            continue;
                        }
                        _ => { println!("\x1b[91mUnknown command: {}\x1b[0m", parts[0]); continue; }
                    }
                }

                messages.push(Message { role: "user".to_string(), content: user_input.to_string() });
                print!("\x1b[92mAssistant:\x1b[0m ");
                io::stdout().flush()?;
                match chat_request(&client, &messages).await {
                    Ok(reply) => {
                        let _ = rl.add_history_entry(user_input);
                        messages.push(Message { role: "assistant".to_string(), content: reply });
                        save_to_file(&hist_path, &messages);
                    }
                    Err(_) => println!("\n\x1b[91mError: Server unreachable.\x1b[0m"),
                }
            },
            Err(_) => break,
        }
    }
    Ok(())
}
