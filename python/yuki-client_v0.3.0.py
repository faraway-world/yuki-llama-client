import json
import requests
import os
import sys
import readline

SERVER_URL = "http://127.0.0.1:8080/v1/chat/completions"
HISTORY_FILE = "history.json"

# ANSI colors to match your Rust output
YELLOW = "\033[93m"
CYAN = "\033[96m"
RESET = "\033[0m"

def load_history():
    """Loads chat history from history.json."""
    if os.path.exists(HISTORY_FILE):
        try:
            with open(HISTORY_FILE, "r") as f:
                msgs = json.load(f)
                print(f"{YELLOW}--- Loaded history ({len(msgs)} messages) ---{RESET}")
                return msgs
        except (json.JSONDecodeError, IOError):
            return []
    return []

def save_history(messages):
    """Saves the current message list to history.json."""
    try:
        with open(HISTORY_FILE, "w") as f:
            json.dump(messages, f, indent=2)
    except IOError as e:
        print(f"\nError saving history: {e}")

def chat(messages):
    """Sends messages to the server and streams the response."""
    payload = {
        "model": "local",
        "messages": messages,
        "stream": True
    }
    
    full_reply = ""
    try:
        # We use stream=True to handle the token-by-token response
        response = requests.post(SERVER_URL, json=payload, stream=True, timeout=300)
        response.raise_for_status()

        for line in response.iter_lines(decode_unicode=True):
            if not line or not line.startswith("data: "):
                continue
            
            data_str = line[6:].strip() # Remove "data: "
            if data_str == "[DONE]":
                break
                
            try:
                chunk = json.loads(data_str)
                content = chunk.get("choices", [{}])[0].get("delta", {}).get("content", "")
                if content:
                    print(content, end="", flush=True)
                    full_reply += content
            except json.JSONDecodeError:
                continue
                
        return full_reply
    except requests.exceptions.RequestException as e:
        print(f"\nError: {e}")
        return None

def main():
    messages = load_history()
    
    print(f"{CYAN}Yuki Chat (Python). Type 'exit' to quit or 'clear' to reset.{RESET}")

    while True:
        try:
            # input() automatically uses the 'readline' module for arrow keys
            user_input = input("\nYou: ").strip()
            
            if not user_input:
                continue
            if user_input.lower() in ["exit", "quit"]:
                break
            
            if user_input.lower() == "clear":
                messages = []
                if os.path.exists(HISTORY_FILE):
                    os.remove(HISTORY_FILE)
                print("History cleared.")
                continue

            # Add user message to state
            messages.append({"role": "user", "content": user_input})

            print("\nAssistant: ", end="", flush=True)
            reply = chat(messages)
            
            if reply is not None:
                messages.append({"role": "assistant", "content": reply})
                save_history(messages)
            print()

        except (EOFError, KeyboardInterrupt):
            print("\nGoodbye!")
            break

if __name__ == "__main__":
    main()
