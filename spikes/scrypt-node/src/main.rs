//! Essai 4 : Rust vérifie les hachages scrypt produits par hashSecret() de Node, et inversement.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use unicode_normalization::UnicodeNormalization;

#[derive(Deserialize, Serialize)]
struct Vector {
    secret: String,
    hash: String,
}

fn derive(secret: &str, salt: &[u8], n: u64, r: u32, p: u32, len: usize) -> Option<Vec<u8>> {
    if !n.is_power_of_two() || n < 2 {
        return None;
    }
    let params = scrypt::Params::new(n.trailing_zeros() as u8, r, p, len).ok()?;
    let normalized: String = secret.nfkc().collect();
    let mut out = vec![0u8; len];
    scrypt::scrypt(normalized.as_bytes(), salt, &params, &mut out).ok()?;
    Some(out)
}

fn hash_secret(secret: &str) -> String {
    let salt: [u8; 16] = rand::random();
    let hash = derive(secret, &salt, 32768, 8, 1, 64).unwrap();
    format!("scrypt$32768$8$1${}${}", STANDARD.encode(salt), STANDARD.encode(hash))
}

fn verify_secret(secret: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [kind, n, r, p, salt, hash] = parts[..] else { return false };
    if kind != "scrypt" {
        return false;
    }
    let (Ok(n), Ok(r), Ok(p), Ok(salt), Ok(expected)) =
        (n.parse(), r.parse(), p.parse(), STANDARD.decode(salt), STANDARD.decode(hash))
    else {
        return false;
    };
    derive(secret, &salt, n, r, p, expected.len()).is_some_and(|actual| actual.ct_eq(&expected).into())
}

fn main() {
    let vectors: Vec<Vector> = serde_json::from_str(include_str!("../vectors.json")).unwrap();
    let mut failures = 0;
    for v in &vectors {
        let ok = verify_secret(&v.secret, &v.hash);
        let wrong = verify_secret(&format!("{}x", v.secret), &v.hash);
        println!("{} {:?} (mauvais secret refusé : {})", if ok { "OK  " } else { "ÉCHEC" }, v.secret, !wrong);
        failures += usize::from(!ok || wrong);
    }
    // NFKC : la forme décomposée doit valider le hachage de la forme précomposée.
    let cross = verify_secret("Cafe\u{301} cre\u{300}me", &vectors[1].hash);
    println!("{} forme décomposée contre hachage précomposé", if cross { "OK  " } else { "ÉCHEC" });
    failures += usize::from(!cross);

    // Sens inverse : hachages Rust, vérifiés ensuite par Node.
    let out: Vec<Vector> =
        vectors.iter().map(|v| Vector { secret: v.secret.clone(), hash: hash_secret(&v.secret) }).collect();
    std::fs::write("rust-vectors.json", serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("{} hachages Rust écrits dans rust-vectors.json", out.len());
    std::process::exit(i32::from(failures > 0));
}
