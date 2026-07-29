//! `noctalia msg` CLI entry point.
//! Port of `src/ipc/cli.{cpp,h}`.

use crate::client;

/// Builds the command line `run_cli` sends, or `None` if `args` is empty (the caller then
/// reports the "requires a command" error). `args` are the tokens after the `msg` verb (the
/// C++'s `argv[2..]`).
///
/// `notification-show` gets special handling identical to the C++: its second and later tokens
/// become a JSON `{"summary", "body"}` payload rather than being forwarded as literal text.
/// nlohmann::json's default object type is `std::map` (key-sorted) and this crate's `serde_json`
/// has no `preserve_order` feature anywhere in its dependency tree (checked via `cargo tree`), so
/// both default to a `BTreeMap`/`std::map`-sorted object — `{"body":...,"summary":...}` either
/// way — verified, not assumed.
fn compose_command_line(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return None;
    }

    if args[0] == "notification-show" && args.len() >= 3 {
        let mut body = args[2].clone();
        for word in &args[3..] {
            body.push(' ');
            body.push_str(word);
        }
        let payload = serde_json::json!({
            "summary": args[1],
            "body": body,
        });
        return Some(format!("{} {payload}", args[0]));
    }

    Some(args.join(" "))
}

/// Entry point for `noctalia msg <command> [args...]`. `args` are the tokens after the `msg`
/// verb; wiring the real process argv into this slice is task 4.2's job (the `clap`-based
/// binary). Returns a process exit code. Forwards the command to the running instance over the
/// IPC socket.
#[must_use]
pub fn run_cli(args: &[String]) -> i32 {
    match compose_command_line(args) {
        Some(line) => client::send(&line),
        None => {
            eprintln!("error: msg requires a command (try: noctalia msg --help)");
            1
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_yield_no_command_line() {
        assert_eq!(compose_command_line(&[]), None);
    }

    #[test]
    fn plain_command_joins_tokens_with_spaces() {
        let args = vec!["panel-toggle".to_string(), "launcher".to_string()];
        assert_eq!(
            compose_command_line(&args),
            Some("panel-toggle launcher".to_string())
        );
    }

    #[test]
    fn notification_show_builds_sorted_json_payload_with_joined_body() {
        let args = vec![
            "notification-show".to_string(),
            "Title".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ];
        assert_eq!(
            compose_command_line(&args),
            Some(r#"notification-show {"body":"hello world","summary":"Title"}"#.to_string())
        );
    }

    #[test]
    fn notification_show_below_arity_falls_back_to_plain_join() {
        // Only "notification-show" + one token: below the C++'s argc >= 5 threshold, so it's
        // just forwarded as-is rather than treated as a summary/body pair.
        let args = vec!["notification-show".to_string(), "onlyone".to_string()];
        assert_eq!(
            compose_command_line(&args),
            Some("notification-show onlyone".to_string())
        );
    }
}
