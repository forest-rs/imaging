// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Build-script configuration for `imaging_vello` tests.

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=IMAGING_CI_GPU_SUPPORT");
    println!("cargo:rustc-check-cfg=cfg(skip_gpu_tests)");

    let Ok(mut value) = env::var("IMAGING_CI_GPU_SUPPORT") else {
        return;
    };
    value.make_ascii_lowercase();

    match value.as_str() {
        "yes" | "y" => {}
        "no" | "n" => {
            println!("cargo:rustc-cfg=skip_gpu_tests");
        }
        _ => {
            println!("cargo:warning=IMAGING_CI_GPU_SUPPORT should be set to yes/y or no/n");
        }
    }
}
