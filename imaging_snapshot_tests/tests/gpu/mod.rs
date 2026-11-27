// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

pub(super) fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let policy = std::env::var("IMAGING_CI_GPU_SUPPORT");
    if policy.as_deref() == Ok("no") {
        return None;
    }
    let result = pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(|err| err.to_string())?;
        adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|err| err.to_string())
    });
    match result {
        Ok(handles) => Some(handles),
        Err(err) if policy.as_deref() == Ok("yes") => panic!("required GPU unavailable: {err}"),
        Err(err) => {
            eprintln!("Skipping GPU test: {err}");
            None
        }
    }
}
