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

pub(crate) mod live;

pub(crate) mod cbor_types;

mod debug_byte_slice;
pub(crate) use debug_byte_slice::DebugByteSlice;

#[allow(dead_code)]
pub mod hash_utils;
pub(crate) mod io_utils;
pub(crate) mod merkle;
pub(crate) mod mime;
#[allow(dead_code)] // for wasm build
pub(crate) mod patch;
#[cfg(feature = "add_thumbnails")]
pub(crate) mod thumbnail;
pub(crate) mod time_it;
#[allow(dead_code)] // for wasm builds
pub(crate) mod xmp_inmemory_utils;
// shared unit testing utilities
#[cfg(test)]
#[allow(dead_code)] // for wasm build
pub mod test;

#[cfg(test)]
pub(crate) mod test_signer;
