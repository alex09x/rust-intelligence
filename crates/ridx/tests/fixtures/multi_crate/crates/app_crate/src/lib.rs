/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! App crate in multi-crate fixture workspace consuming core_lib, two versions of calc, and ext_service.

use core_lib::CoreService as ExternalCoreService;

pub struct CoreService {
    pub local_val: u32,
}

impl CoreService {
    pub fn execute(&self) -> u32 {
        self.local_val * 2
    }
}

pub fn invoke_service() -> u32 {
    let ext_svc = ExternalCoreService::new(42);
    let local_svc = CoreService { local_val: 10 };
    ext_svc.execute() + local_svc.execute()
}

pub fn invoke_two_versions() -> u32 {
    calc_v1::calculate() + calc_v2::calculate()
}

pub fn invoke_external() -> u32 {
    ext_service::external_compute()
}
