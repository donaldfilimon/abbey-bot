//! Pure native-player commands. User text is an argv value, never AppleScript source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Player { Spotify, Music }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script { pub source: &'static str, pub argument: String }

pub fn play(player: Player, query: &str) -> Result<Script, &'static str> {
    let query = query.trim();
    if query.len() > 500 || query.chars().any(char::is_control) { return Err("Use a query under 500 bytes without control characters."); }
    let source = match player {
        Player::Spotify => {
            if !query.is_empty() && !(query.strip_prefix("spotify:track:").is_some_and(|id| id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()))) {
                return Err("Spotify accepts a spotify:track: URI, or an empty query to mirror the current selection. Use Music for library text search.");
            }
            "on run argv\ntell application id \"com.spotify.client\"\nif item 1 of argv is \"\" then\nplay\nelse\nplay track (item 1 of argv)\nend if\nend tell\nend run"
        }
        Player::Music => "on run argv\ntell application id \"com.apple.Music\"\nif item 1 of argv is \"\" then\nplay\nelse\nset matches to search library playlist 1 for (item 1 of argv) only songs\nif (count of matches) is 0 then error \"No matching library track\"\nplay item 1 of matches\nend if\nend tell\nend run",
    };
    Ok(Script { source, argument: query.into() })
}

pub fn pause(player: Player) -> Script {
    Script { source: match player {
        Player::Spotify => "on run argv\ntell application id \"com.spotify.client\" to pause\nend run",
        Player::Music => "on run argv\ntell application id \"com.apple.Music\" to pause\nend run",
    }, argument: String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn music_query_cannot_become_code() {
        let query = "\" & do shell script \"touch /tmp/no\"";
        let script = play(Player::Music, query).unwrap();
        assert!(!script.source.contains(query)); assert_eq!(script.argument, query);
        assert!(script.source.contains("search library playlist 1 for (item 1 of argv) only songs"));
    }
    #[test] fn spotify_and_pause_golden() {
        assert_eq!(pause(Player::Spotify).source, "on run argv\ntell application id \"com.spotify.client\" to pause\nend run");
        assert_eq!(pause(Player::Music).source, "on run argv\ntell application id \"com.apple.Music\" to pause\nend run");
        assert!(play(Player::Spotify,"spotify:track:0123456789ABCDEFGHIJKL").is_ok());
        for query in ["search terms", "spotify:track:abc", "https://example.com", "\nattack\nx"] { assert!(play(Player::Spotify,query).is_err()); }
        assert!(play(Player::Music,&"x".repeat(501)).is_err());
        assert!(play(Player::Spotify, "").is_ok());
    }
}
