// Copyright 2024 Adobe. All rights reserved.
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

#![allow(unused)] // TEMPORARY while updating ...

use std::fmt::{Debug, Error, Formatter};

pub(crate) struct DebugByteSlice<'a>(pub(crate) &'a [u8]);

impl Debug for DebugByteSlice<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        if self.0.len() > 20 {
            write!(
                f,
                "{} bytes starting with {:02x?}",
                self.0.len(),
                &self.0[0..20]
            )
        } else {
            write!(f, "{:02x?}", self.0)
        }
    }
}
