#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
use sp_externalities::ExternalitiesExt;

#[cfg(feature = "std")]
sp_externalities::decl_extension! {
    pub struct OffworkerExt(Box<dyn OffworkerExtension>);
}

#[cfg(feature = "std")]
impl OffworkerExt {
    pub fn new<T: OffworkerExtension>(t: T) -> Self {
        Self(Box::new(t))
    }
}

#[cfg(feature = "std")]
/// (Decrypted Weights, Key)
pub type DecryptedWeights = (Vec<(u16, u16)>, Vec<u8>);

#[cfg(feature = "std")]
pub type EncryptionKey = (Vec<u8>, Vec<u8>);

#[cfg(not(feature = "std"))]
/// (Decrypted Weights, Key)
pub type DecryptedWeights = (sp_std::vec::Vec<(u16, u16)>, sp_std::vec::Vec<u8>);

#[cfg(not(feature = "std"))]
pub type EncryptionKey = (sp_std::vec::Vec<u8>, sp_std::vec::Vec<u8>);

#[cfg(feature = "std")]
pub trait OffworkerExtension: Send + 'static {
    fn decrypt_weight(&self, encrypted: Vec<u8>) -> Option<DecryptedWeights>;

    fn is_decryption_node(&self) -> bool;

    fn get_encryption_key(&self) -> Option<EncryptionKey>;
}

#[sp_runtime_interface::runtime_interface]
pub trait Offworker {
    fn decrypt_weight(&mut self, encrypted: sp_std::vec::Vec<u8>) -> Option<DecryptedWeights> {
        self.extension::<OffworkerExt>()
            .expect("missing offworker ext")
            .decrypt_weight(encrypted)
    }

    fn is_decryption_node(&mut self) -> bool {
        self.extension::<OffworkerExt>()
            .expect("missing offworker ext")
            .is_decryption_node()
    }

    fn get_encryption_key(&mut self) -> Option<EncryptionKey> {
        self.extension::<OffworkerExt>()
            .expect("missing offworker ext")
            .get_encryption_key()
    }
}
