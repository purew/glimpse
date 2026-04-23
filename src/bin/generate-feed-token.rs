/// Generate a feed token for a user and print the hash to store in users.toml.
///
/// Usage: cargo run --bin generate-feed-token
///
/// Output:
///   Token (use in feed URL):                <token>
///   Hash  (add to users.toml):              <hash>
///
/// 1. Run this binary; copy both values.
/// 2. Add `feed_token_hash = "<hash>"` to the user's entry in users.toml.
/// 3. Share the feed URL with the user:
///    `https://yoursite.com/feed/<token>.xml`
use argon2::password_hash::rand_core::{OsRng, RngCore};
use sha2::Digest;

fn main() {
    let mut bytes = [0u8; 16]; // 128 bits of entropy
    OsRng.fill_bytes(&mut bytes);

    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hash_bytes = sha2::Sha256::digest(token.as_bytes());
    let hash: String = hash_bytes.iter().map(|b| format!("{b:02x}")).collect();

    println!("Token (use in feed URL):   {token}");
    println!("Hash  (add to users.toml): {hash}");
}
