use ow_extensions::OffworkerExtension;
use rand::rngs::OsRng;
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey},
    traits::PublicKeyParts,
    BigUint, Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey,
};
use sp_core::{sr25519, Pair};
use sp_keystore::{testing::MemoryKeystore, Keystore};
use sp_runtime::KeyTypeId;
use std::io::{Cursor, Read};

pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"wcs!");

pub struct MockOffworkerExt {
    pub key: Option<rsa::RsaPrivateKey>,
}

impl Default for MockOffworkerExt {
    fn default() -> Self {
        let keystore = MemoryKeystore::new();

        // Generate a new key pair and add it to the keystore
        let (pair, _, _) = sr25519::Pair::generate_with_phrase(None);
        let public = pair.public();
        keystore
            .sr25519_generate_new(
                KEY_TYPE,
                Some(&format!("//{}", hex::encode(public.as_ref() as &[u8]))),
            )
            .expect("Failed to add key to keystore");

        // Generate an RSA key pair
        let mut rng = OsRng;
        let bits = 2048;
        let private_key = RsaPrivateKey::new(&mut rng, bits).expect("Failed to generate RSA key");
        let public_key = RsaPublicKey::from(&private_key);

        // Store the RSA public key components in the keystore
        let n = public_key.n().to_bytes_be();
        let e = public_key.e().to_bytes_be();
        let combined_key = [n, e].concat();
        keystore
            .insert(KEY_TYPE, "rsa_public_key", &combined_key)
            .expect("Failed to store RSA public key");

        Self {
            key: Some(private_key),
        }
    }
}

impl ow_extensions::OffworkerExtension for MockOffworkerExt {
    // 1. Here we switch the RSA for elgamal
    fn decrypt_weight(&self, encrypted: Vec<u8>) -> Option<(Vec<(u16, u16)>, Vec<u8>)> {
        let Some(key) = &self.key else {
            return None;
        };

        let Some(vec) = encrypted
            .chunks(key.size())
            .map(|chunk| match key.decrypt(Pkcs1v15Encrypt, &chunk) {
                Ok(decrypted) => Some(decrypted),
                Err(_) => None,
            })
            .collect::<Option<Vec<Vec<u8>>>>()
        else {
            return None;
        };

        let decrypted = vec.into_iter().flatten().collect::<Vec<_>>();

        let mut weights = Vec::new();

        let mut cursor = Cursor::new(&decrypted);

        let Some(length) = read_u32(&mut cursor) else {
            return None;
        };
        for _ in 0..length {
            let Some(uid) = read_u16(&mut cursor) else {
                return None;
            };

            let Some(weight) = read_u16(&mut cursor) else {
                return None;
            };

            weights.push((uid, weight));
        }

        let mut key = Vec::new();
        cursor.read_to_end(&mut key).ok()?;

        Some((weights, key))
    }

    fn is_decryption_node(&self) -> bool {
        self.key.is_some()
    }

    fn get_encryption_key(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        let Some(key) = &self.key else {
            return None;
        };

        let public = rsa::RsaPublicKey::from(key);
        Some((public.n().to_bytes_be(), public.e().to_bytes_be()))
    }
}

fn read_u32(cursor: &mut Cursor<&Vec<u8>>) -> Option<u32> {
    let mut buf: [u8; 4] = [0u8; 4];
    match cursor.read_exact(&mut buf[..]) {
        Ok(()) => Some(u32::from_be_bytes(buf)),
        Err(_) => None,
    }
}

fn read_u16(cursor: &mut Cursor<&Vec<u8>>) -> Option<u16> {
    let mut buf = [0u8; 2];
    match cursor.read_exact(&mut buf[..]) {
        Ok(()) => Some(u16::from_be_bytes(buf)),
        Err(_) => None,
    }
}

pub fn hash(data: Vec<(u16, u16)>) -> Vec<u8> {
    //can be any sha256 lib, this one is used by substrate.
    // dbg!(data.clone());
    sp_io::hashing::sha2_256(&weights_to_blob(&data.clone()[..])[..]).to_vec()
}

fn weights_to_blob(weights: &[(u16, u16)]) -> Vec<u8> {
    let mut encoded = Vec::new();
    encoded.extend((weights.len() as u32).to_be_bytes());
    encoded.extend(weights.iter().flat_map(|(uid, weight)| {
        vec![uid.to_be_bytes(), weight.to_be_bytes()].into_iter().flat_map(|a| a)
    }));

    encoded
}

// the key needs to be retrieved from the blockchain
pub fn encrypt(key: (Vec<u8>, Vec<u8>), data: Vec<(u16, u16)>, validator_key: Vec<u8>) -> Vec<u8> {
    let rsa_key = RsaPublicKey::new(
        BigUint::from_bytes_be(&key.0),
        BigUint::from_bytes_be(&key.1),
    )
    .expect("Failed to create RSA key");

    let encoded = [
        (data.len() as u32).to_be_bytes().to_vec(),
        data.into_iter()
            .flat_map(|(uid, weight)| {
                uid.to_be_bytes().into_iter().chain(weight.to_be_bytes().into_iter())
            })
            .collect(),
        validator_key,
    ]
    .concat();

    let max_chunk_size = rsa_key.size() - 11; // 11 bytes for PKCS1v15 padding

    encoded
        .chunks(max_chunk_size)
        .flat_map(|chunk| {
            rsa_key.encrypt(&mut OsRng, Pkcs1v15Encrypt, chunk).expect("Encryption failed")
        })
        .collect()
}

#[test]
fn encrypt_and_decrypt() {
    // use rsa::traits::PrivateKeyParts;

    let weights = vec![(1, 2), (3, 4)];
    let validator_key = vec![11, 22, 33, 44];

    let rsa_key = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
    let mock_offworker_ext = MockOffworkerExt {
        key: Some(rsa_key.clone()),
    };

    let rsa_key_pem = rsa_key.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();

    let pub_key = rsa_key.to_public_key();
    let pub_key_tp = (pub_key.n().to_bytes_be(), pub_key.e().to_bytes_be());

    let pub_key_n_hex = hex::encode(&pub_key_tp.0);
    let pub_key_e_hex = hex::encode(&pub_key_tp.1);

    println!("weights = {:?}", weights);
    println!("validator_key = {:?}", validator_key);

    println!("pub_key_n_hex = {:?}", pub_key_n_hex);
    println!("pub_key_e_hex = {:?}", pub_key_e_hex);

    println!("rsa_key_pem = {:?}", rsa_key_pem);

    let encrypted = encrypt(pub_key_tp, weights.clone(), validator_key.clone());

    let encrypted_hex = hex::encode(&encrypted);
    println!("encrypted_hex = {:?}", encrypted_hex);

    let (decrypted_weights, decrypted_key) = mock_offworker_ext.decrypt_weight(encrypted).unwrap();

    assert_eq!(decrypted_weights, weights.clone());
    assert_eq!(decrypted_key, validator_key.clone());
}

#[test]
fn decrypt_external() {
    let weights = vec![(1, 2), (3, 4)];
    let validator_key = vec![11, 22, 33, 44];

    // Encrypted data from Python implementation
    let encrypted = "62724424d1ca39a8873b391ca7feb3fde3b2676f19283a42b2afa13544f987eba8124b1ce494ebc51e8afe3a1ef4326713b774928c0034d45c7af85f9c2e6f5b0c33cb53074a403d44892da60bd78672bc223714f96c4eeb877ffc1088b249cedfd3ae40d892d86696d518eb7c20feffa7dfd8b55c80113106bdcc2cce4ab09f";
    let encrypted = hex::decode(encrypted).unwrap();

    let rsa_key_pem = "
-----BEGIN RSA PRIVATE KEY-----
MIICXQIBAAKBgQDXQNAmQOmL78ISODmSBenbpbcR0jfQbfBswq9vksOddikukMNN
BJOeOj4YUgSCt2K+auiFnw+fE9B1sACoiSv8AiVym8n82E0sQUk0cjFVf0Z4JBqv
PAgKR8M6ofTJCu58rPaU3e6+mru/Ixqf26QQr7vI96Pp93aiZQTOShmC6QIDAQAB
AoGBAIixPf2s5yLYZLPRRK34V2QGvlTw3ETeK/nFQEdoOhT6fnh1sbBtIZkvf1NO
clLYRjqKBZMlSXRJzu2NkT11rpm1hTTuc99w0SjZDHFpj0TppXtagmJYwHBYt5Ac
oNan6ALTlUbxEHtIj4rGghJAJBOVTq0pi8PdVgAQgq3cArUBAkEA2f9SFOmDWN7w
PO6yHZfj7e8i65W8v4HZXV/EWv3kCZW5KZsM3OBlqqx1txIljxF146C7ZpBLLQEK
ubVOqKqPsQJBAPzHBuczD6GziSbN9sjgj4sAxGwExp8Z747rxGVlB56ak68aqFt1
GDuwib0NIrrDUuGlQUKIWUm6amSwu/UJbLkCQDsZS8Bdmf0y20A5mdIKBoHPrdDe
VEA6zJnSx6G/aN3sWDleTntm3kkJ3hPWeJYzrpkaTxO8FJVLzgOQkpWJP9ECQQD1
q0EsRlX05BZx3k7w4D7h67b6/JFFY+GNV9qiaNRE8xqBXjkt2dnZeTQExtVwChFt
ODz6uqV8oG5yucmS1rwRAkA1KjcZDPBRZ05wlf8VZuJjWYIRbVx3PBpQJPbtW7Vg
fvRuW5JF+WZtGddyU4751JNNNhmwbwGmsmphy7EOHHaC
-----END RSA PRIVATE KEY-----";

    let rsa_key = RsaPrivateKey::from_pkcs1_pem(rsa_key_pem).unwrap();
    let mock_offworker_ext = MockOffworkerExt { key: Some(rsa_key) };

    println!("weights = {:?}", weights);
    println!("validator_key = {:?}", validator_key);

    let (decrypted_weights, decrypted_key) = mock_offworker_ext.decrypt_weight(encrypted).unwrap();

    assert_eq!(decrypted_weights, weights.clone());
    assert_eq!(decrypted_key, validator_key.clone());
}
