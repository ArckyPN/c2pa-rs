// Copyright 2022 Adobe. All rights reserved.
// This file is licensed to you under the Apache License,
// Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0)
// or the MIT license (http://opensource.org/licenses/MIT),
// at your option.

// Unless required by applicable law or agreed to in writing,
// this software is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR REPRESENTATIONS OF ANY KIND, either express or
// implied. See the LICENSE-MIT and LICENSE-APACHE files for the
// specific language governing permissions and limitations under
// each license.

use c2pa_crypto::{
    time_stamp::{AsyncTimeStampProvider, TimeStampError, TimeStampProvider},
    SigningAlg,
};

use crate::{DynamicAssertion, Result};
/// The `Signer` trait generates a cryptographic signature over a byte array.
///
/// This trait exists to allow the signature mechanism to be extended.
pub trait Signer: TimeStampProvider {
    /// Returns a new byte array which is a signature over the original.
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>>;

    /// Returns the algorithm of the Signer.
    fn alg(&self) -> SigningAlg;

    /// Returns the certificates as a Vec containing a Vec of DER bytes for each certificate.
    fn certs(&self) -> Result<Vec<Vec<u8>>>;

    /// Returns the size in bytes of the largest possible expected signature.
    /// Signing will fail if the result of the `sign` function is larger
    /// than this value.
    fn reserve_size(&self) -> usize;

    /// OCSP response for the signing cert if available
    /// This is the only C2PA supported cert revocation method.
    /// By pre-querying the value for a your signing cert the value can
    /// be cached taking pressure off of the CA (recommended by C2PA spec)
    fn ocsp_val(&self) -> Option<Vec<u8>> {
        None
    }

    /// If this returns true the sign function is responsible for for direct handling of the COSE structure.
    ///
    /// This is useful for cases where the signer needs to handle the COSE structure directly.
    /// Not recommended for general use.
    fn direct_cose_handling(&self) -> bool {
        false
    }

    /// Returns a list of dynamic assertions that should be included in the manifest.
    fn dynamic_assertions(&self) -> Vec<Box<dyn DynamicAssertion>> {
        Vec::new()
    }
}

/// Trait to allow loading of signing credential from external sources
#[allow(dead_code)] // this here for wasm builds to pass clippy  (todo: remove)
pub(crate) trait ConfigurableSigner: Signer + Sized {
    /// Create signer form credential files
    #[cfg(feature = "file_io")]
    fn from_files<P: AsRef<std::path::Path>>(
        signcert_path: P,
        pkey_path: P,
        alg: SigningAlg,
        tsa_url: Option<String>,
    ) -> Result<Self> {
        use crate::Error;

        let signcert = std::fs::read(signcert_path).map_err(Error::IoError)?;
        let pkey = std::fs::read(pkey_path).map_err(Error::IoError)?;

        Self::from_signcert_and_pkey(&signcert, &pkey, alg, tsa_url)
    }

    /// Create signer from credentials data
    fn from_signcert_and_pkey(
        signcert: &[u8],
        pkey: &[u8],
        alg: SigningAlg,
        tsa_url: Option<String>,
    ) -> Result<Self>;
}

use async_trait::async_trait;

/// The `AsyncSigner` trait generates a cryptographic signature over a byte array.
///
/// This trait exists to allow the signature mechanism to be extended.
///
/// Use this when the implementation is asynchronous.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait AsyncSigner: Sync + AsyncTimeStampProvider {
    /// Returns a new byte array which is a signature over the original.
    async fn sign(&self, data: Vec<u8>) -> Result<Vec<u8>>;

    /// Returns the algorithm of the Signer.
    fn alg(&self) -> SigningAlg;

    /// Returns the certificates as a Vec containing a Vec of DER bytes for each certificate.
    fn certs(&self) -> Result<Vec<Vec<u8>>>;

    /// Returns the size in bytes of the largest possible expected signature.
    /// Signing will fail if the result of the `sign` function is larger
    /// than this value.
    fn reserve_size(&self) -> usize;

    /// OCSP response for the signing cert if available
    /// This is the only C2PA supported cert revocation method.
    /// By pre-querying the value for a your signing cert the value can
    /// be cached taking pressure off of the CA (recommended by C2PA spec)
    async fn ocsp_val(&self) -> Option<Vec<u8>> {
        None
    }

    /// If this returns true the sign function is responsible for for direct handling of the COSE structure.
    ///
    /// This is useful for cases where the signer needs to handle the COSE structure directly.
    /// Not recommended for general use.
    fn direct_cose_handling(&self) -> bool {
        false
    }

    /// Returns a list of dynamic assertions that should be included in the manifest.
    fn dynamic_assertions(&self) -> Vec<Box<dyn DynamicAssertion>> {
        Vec::new()
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
pub trait AsyncSigner: AsyncTimeStampProvider {
    /// Returns a new byte array which is a signature over the original.
    async fn sign(&self, data: Vec<u8>) -> Result<Vec<u8>>;

    /// Returns the algorithm of the Signer.
    fn alg(&self) -> SigningAlg;

    /// Returns the certificates as a Vec containing a Vec of DER bytes for each certificate.
    fn certs(&self) -> Result<Vec<Vec<u8>>>;

    /// Returns the size in bytes of the largest possible expected signature.
    /// Signing will fail if the result of the `sign` function is larger
    /// than this value.
    fn reserve_size(&self) -> usize;

    /// OCSP response for the signing cert if available
    /// This is the only C2PA supported cert revocation method.
    /// By pre-querying the value for a your signing cert the value can
    /// be cached taking pressure off of the CA (recommended by C2PA spec)
    async fn ocsp_val(&self) -> Option<Vec<u8>> {
        None
    }

    /// If this returns true the sign function is responsible for for direct handling of the COSE structure.
    ///
    /// This is useful for cases where the signer needs to handle the COSE structure directly.
    /// Not recommended for general use.
    fn direct_cose_handling(&self) -> bool {
        false
    }

    /// Returns a list of dynamic assertions that should be included in the manifest.
    fn dynamic_assertions(&self) -> Vec<Box<dyn DynamicAssertion>> {
        Vec::new()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait RemoteSigner: Sync {
    /// Returns the `CoseSign1` bytes signed by the [`RemoteSigner`].
    ///
    /// The size of returned `Vec` must match the value returned by `reserve_size`.
    /// This data will be embedded in the JUMBF `c2pa.signature` box of the manifest.
    /// `data` are the bytes of the claim to be remotely signed.
    async fn sign_remote(&self, data: &[u8]) -> Result<Vec<u8>>;

    /// Returns the size in bytes of the largest possible expected signature.
    ///
    /// Signing will fail if the result of the `sign` function is larger
    /// than this value.
    fn reserve_size(&self) -> usize;
}

impl Signer for Box<dyn Signer + Send + Sync> {
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        (**self).sign(data)
    }

    fn alg(&self) -> SigningAlg {
        (**self).alg()
    }

    fn certs(&self) -> Result<Vec<Vec<u8>>> {
        (**self).certs()
    }

    fn reserve_size(&self) -> usize {
        (**self).reserve_size()
    }

    fn ocsp_val(&self) -> Option<Vec<u8>> {
        (**self).ocsp_val()
    }

    fn direct_cose_handling(&self) -> bool {
        (**self).direct_cose_handling()
    }

    fn dynamic_assertions(&self) -> Vec<Box<dyn DynamicAssertion>> {
        (**self).dynamic_assertions()
    }
}

impl TimeStampProvider for Box<dyn Signer + Send + Sync> {
    fn time_stamp_service_url(&self) -> Option<String> {
        (**self).time_stamp_service_url()
    }

    fn time_stamp_request_headers(&self) -> Option<Vec<(String, String)>> {
        (**self).time_stamp_request_headers()
    }

    fn time_stamp_request_body(
        &self,
        message: &[u8],
    ) -> std::result::Result<Vec<u8>, TimeStampError> {
        (**self).time_stamp_request_body(message)
    }

    fn send_time_stamp_request(
        &self,
        message: &[u8],
    ) -> Option<std::result::Result<Vec<u8>, TimeStampError>> {
        (**self).send_time_stamp_request(message)
    }
}
