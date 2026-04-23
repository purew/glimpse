/// Print an argon2 hash for a password supplied as the first argument.
///
/// Usage: cargo run --bin hash-password -- <password>
use argon2::{
    Argon2, PasswordHasher,
    password_hash::{SaltString, rand_core::OsRng},
};

fn main() {
    let password = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: hash-password <password>");
        std::process::exit(1);
    });

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("hashing failed");
    println!("{hash}");
}
