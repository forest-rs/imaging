// Copyright 2026 the Imaging Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Snapshot test infrastructure for `imaging` backends.
//!
//! This crate will host Kompari-driven image snapshot tests (borrowed from Understory's
//! `understory_imaging_snapshot_tests`) while keeping the `imaging` core crate dependency-free.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use imaging as _;
use svg_imaging as _;

#[cfg(feature = "vello_cpu")]
use imaging_vello_cpu as _;

#[cfg(feature = "skia")]
use imaging_skia as _;

#[cfg(feature = "tiny_skia")]
use imaging_tiny_skia as _;

#[cfg(feature = "vello_hybrid")]
use imaging_vello_hybrid as _;

#[cfg(feature = "vello_hybrid")]
use vello_hybrid as _;

#[cfg(feature = "vello")]
use imaging_vello as _;

pub mod cases;
