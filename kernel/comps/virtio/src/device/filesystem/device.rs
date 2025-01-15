// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::{fmt::Debug, iter::Fuse};

use log::debug;
use ostd::{
    early_print, early_println,
    mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmReader, VmWriter},
    sync::{RwLock, SpinLock},
    trap::TrapFrame,
    Pod,
};

use super::{
    config::{FilesystemFeatures, VirtioFilesystemConfig},
    fuse::*,
    request::{fuse_pad_str, AnyFuseDevice, FuseReaddirOut},
};
use crate::{
    device::VirtioDeviceError,
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

pub struct FilesystemDevice {
    config_manager: ConfigManager<VirtioFilesystemConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,

    hiprio_queue: SpinLock<VirtQueue>,
    request_queues: Vec<SpinLock<VirtQueue>>,
    // notify_queue: SpinLock<VirtQueue>,
    hiprio_buffer: DmaStream,
    request_buffers: Vec<DmaStream>,
    // notify_buffer: DmaStream,
    // callbacks: RwLock<Vec<&'static FilesystemCallback>, LocalIrqDisabled>,
}

impl AnyFuseDevice for FilesystemDevice {
    fn init(&self) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseInitIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseInit as u32,
            unique: 0,
            nodeid: 0,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let initin = FuseInitIn {
            major: FUSE_KERNEL_VERSION,
            minor: FUSE_KERNEL_MINOR_VERSION,
            max_readahead: 0,
            flags: FuseInitFlags::FUSE_INIT_EXT.bits() as u32,
            flags2: 0,
            unused: [0u32; 11],
        };

        let headerin_bytes = headerin.as_bytes();
        let initin_bytes = initin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let initout_bytes = [0u8; 256];
        let concat_req = [
            headerin_bytes,
            initin_bytes,
            &headerout_buffer,
            &initout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseInitIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn opendir(&self, nodeid: u64, flags: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseOpenIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseOpendir as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let openin = FuseOpenIn {
            flags: flags,
            open_flags: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let openin_bytes = openin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let openout_bytes = [0u8; size_of::<FuseOpenOut>()];
        let concat_req = [
            headerin_bytes,
            openin_bytes,
            &headerout_buffer,
            &openout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseOpenIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn readdir(&self, nodeid: u64, fh: u64, offset: u64, size: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseReadIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseReaddir as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let readin = FuseReadIn {
            fh: fh,
            offset: offset,
            size: size,
            read_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let readin_bytes = readin.as_bytes();
        // let readin_bytes = [0u8; 36];
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let readout_bytes = [0u8; 1024];
        let concat_req = [
            headerin_bytes,
            &readin_bytes,
            &headerout_buffer,
            &readout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseReadIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn read(&self, nodeid: u64, fh: u64, offset: u64, size: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseReadIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseRead as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let readin = FuseReadIn {
            fh: fh,
            offset: offset,
            size: size,
            read_flags: 0,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let readin_bytes = readin.as_bytes();
        // let readin_bytes = [0u8; 36];
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let readout_bytes = [0u8; 1024];
        let concat_req = [
            headerin_bytes,
            &readin_bytes,
            &headerout_buffer,
            &readout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseReadIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn open(&self, nodeid: u64, flags: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseOpenIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseOpen as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let openin = FuseOpenIn {
            flags: flags,
            open_flags: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let openin_bytes = openin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let openout_bytes = [0u8; size_of::<FuseOpenOut>()];
        let concat_req = [
            headerin_bytes,
            openin_bytes,
            &headerout_buffer,
            &openout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseOpenIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn flush(&self, nodeid: u64, fh: u64, lock_owner: u64) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseFlushIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseFlush as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let flushin = FuseFlushIn {
            fh: fh,
            lock_owner: lock_owner,
            padding: 0,
            unused: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let flushin_bytes = flushin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        // let flushout_bytes = [0u8; size_of::<FuseFlushOut>()];
        let concat_req = [
            headerin_bytes,
            flushin_bytes,
            &headerout_buffer,
            // &flushout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseFlushIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn releasedir(&self, nodeid: u64, fh: u64, flags: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseReleaseIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseReleasedir as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let releasein = FuseReleaseIn {
            fh: fh,
            flags: flags,
            release_flags: 0,
            lock_owner: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let releasein_bytes = releasein.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        // let releaseout_bytes = [0u8; size_of::<FuseReleaseOut>()];
        let concat_req = [
            headerin_bytes,
            releasein_bytes,
            &headerout_buffer,
            // &releaseout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseReleaseIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn getattr(&self, nodeid: u64, fh: u64, flags: u32, dummy: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseGetattrIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseGetattr as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let getattrin = FuseGetattrIn {
            getattr_flags: flags,
            dummy: dummy,
            fh: fh,
        };

        let headerin_bytes = headerin.as_bytes();
        let getattrin_bytes = getattrin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let getattrout_bytes = [0u8; size_of::<FuseAttrOut>()];
        let concat_req = [
            headerin_bytes,
            getattrin_bytes,
            &headerout_buffer,
            &getattrout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseGetattrIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn setattr(
        &self,
        nodeid: u64,
        valid: u32,
        fh: u64,
        size: u64,
        lock_owner: u64,
        atime: u64,
        mtime: u64,
        ctime: u64,
        atimensec: u32,
        mtimensec: u32,
        ctimensec: u32,
        mode: u32,
        uid: u32,
        gid: u32,
    ) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseSetattrIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseSetattr as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let setattrin = FuseSetattrIn {
            valid: valid,
            padding: 0,
            fh: fh,
            size: size,
            lock_owner: lock_owner,
            atime: atime,
            mtime: mtime,
            ctime: ctime,
            atimensec: atimensec,
            mtimensec: mtimensec,
            ctimensec: ctimensec,
            mode: mode,
            unused4: 0,
            uid: uid,
            gid: gid,
            unused5: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let setattrin_bytes = setattrin.as_bytes();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let setattrout_bytes = [0u8; size_of::<FuseAttrOut>()];
        let concat_req = [
            headerin_bytes,
            setattrin_bytes,
            &headerout_buffer,
            &setattrout_bytes,
        ];

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseSetattrIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn lookup(&self, nodeid: u64, name: Vec<u8>) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        // // add terminating '\0' to the name
        // let mut name = name;
        // name.push(0);

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseInHeader>() as u32 + prepared_name.len() as u32),
            opcode: FuseOpcode::FuseLookup as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let lookupin_bytes = prepared_name.as_slice();

        // early_println!("lookup name: {:?}", name);
        // early_println!("headerin_bytes: {:?}", headerin_bytes);
        // early_println!("lookupin_bytes: {:?}", lookupin_bytes);

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let lookupout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            lookupin_bytes,
            &headerout_buffer,
            &lookupout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn release(&self, nodeid: u64, fh: u64, flags: u32, lock_owner: u64, flush: bool) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseReleaseIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseRelease as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let releasein = FuseReleaseIn {
            fh: fh,
            flags: flags,
            release_flags: if flush { FUSE_RELEASE_FLUSH } else { 0 },
            lock_owner: lock_owner,
        };

        let headerin_bytes = headerin.as_bytes();
        let releasein_bytes = releasein.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        // let releaseout_bytes = [0u8; size_of::<FuseReleaseOut>()];
        let concat_req = [
            headerin_bytes,
            releasein_bytes,
            &headerout_buffer,
            // &releaseout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseReleaseIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn access(&self, nodeid: u64, mask: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseAccessIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseAccess as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let accessin = FuseAccessIn {
            mask: mask,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let accessin_bytes = accessin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let accessout_bytes = [0u8; size_of::<FuseAttrOut>()];
        let concat_req = [
            headerin_bytes,
            accessin_bytes,
            &headerout_buffer,
            &accessout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseAccessIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn statfs(&self, nodeid: u64) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseStatfs as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let statfsout_bytes = [0u8; size_of::<FuseStatfsOut>()];
        let concat_req = [headerin_bytes, &headerout_buffer, &statfsout_bytes].concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn interrupt(&self, nodeid: u64, unique: u64) {
        let mut hiprio_queue = self.hiprio_queue.disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseInterruptIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseInterrupt as u32,
            unique: unique,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let interruptin = FuseInterruptIn { unique: unique };

        let headerin_bytes = headerin.as_bytes();
        let interruptin_bytes = interruptin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let concat_req = [headerin_bytes, interruptin_bytes, &headerout_buffer].concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseInterruptIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        hiprio_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if hiprio_queue.should_notify() {
            hiprio_queue.notify();
        }
    }

    fn mkdir(&self, nodeid: u64, mode: u32, umask: u32, name: Vec<u8>) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseMkdirIn>() as u32
                + prepared_name.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseMkdir as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let mkdirin = FuseMkdirIn {
            mode: mode,
            umask: umask,
        };

        let headerin_bytes = headerin.as_bytes();
        let mkdirin_bytes = mkdirin.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let mkdirout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            mkdirin_bytes,
            prepared_name_bytes,
            &headerout_buffer,
            &mkdirout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len() + size_of::<FuseMkdirIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn create(&self, nodeid: u64, name: Vec<u8>, mode: u32, umask: u32, flags: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseCreateIn>() as u32
                + prepared_name.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseCreate as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let createin = FuseCreateIn {
            flags: flags,
            mode: mode,
            umask: umask,
            open_flags: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let createin_bytes = createin.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let createout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            createin_bytes,
            prepared_name_bytes,
            &headerout_buffer,
            &createout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len() + size_of::<FuseCreateIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn destroy(&self, nodeid: u64) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseDestroy as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let concat_req = [headerin_bytes, &headerout_buffer].concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn rename(&self, nodeid: u64, name: Vec<u8>, newdir: u64, newname: Vec<u8>) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);
        let prepared_newname = fuse_pad_str(&String::from_utf8(newname).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseRenameIn>() as u32
                + prepared_name.len() as u32
                + prepared_newname.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseRename as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let renamein = FuseRenameIn { newdir: newdir };

        let headerin_bytes = headerin.as_bytes();
        let renamein_bytes = renamein.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();
        let prepared_newname_bytes = prepared_newname.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let renameout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            renamein_bytes,
            prepared_name_bytes,
            prepared_newname_bytes,
            &headerout_buffer,
            &renameout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len()
            + prepared_newname.len()
            + size_of::<FuseRenameIn>()
            + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn rename2(&self, nodeid: u64, name: Vec<u8>, newdir: u64, newname: Vec<u8>, flags: u32) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);
        let prepared_newname = fuse_pad_str(&String::from_utf8(newname).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseRename2In>() as u32
                + prepared_name.len() as u32
                + prepared_newname.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseRename2 as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let rename2in = FuseRename2In {
            newdir: newdir,
            flags: flags,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let rename2in_bytes = rename2in.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();
        let prepared_newname_bytes = prepared_newname.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let rename2out_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            rename2in_bytes,
            prepared_name_bytes,
            prepared_newname_bytes,
            &headerout_buffer,
            &rename2out_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len()
            + prepared_newname.len()
            + size_of::<FuseRename2In>()
            + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();
    }

    fn write(&self, nodeid: u64, fh: u64, offset: u64, data: &[u8]) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let data = [data, vec![0u8; (8 - (data.len() & 0x7)) & 0x7].as_slice()].concat();

        let headerin = FuseInHeader {
            len: size_of::<FuseInHeader>() as u32
                + size_of::<FuseWriteIn>() as u32
                + data.len() as u32,
            opcode: FuseOpcode::FuseWrite as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let writein = FuseWriteIn {
            fh: fh,
            offset: offset,
            size: data.len() as u32,
            write_flags: FUSE_WRITE_LOCKOWNER,
            lock_owner: 0,
            flags: 0,
            padding: 0,
        };

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let writeout_buffer = [0u8; size_of::<FuseWriteOut>()];

        let data_bytes = data.as_slice();
        let writein_bytes = writein.as_bytes();
        let headerin_bytes = headerin.as_bytes();
        let concat_req = [
            headerin_bytes,
            writein_bytes,
            data_bytes,
            &headerout_buffer,
            &writeout_buffer,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseWriteIn>() + size_of::<FuseInHeader>() + data.len() as usize;

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in as usize);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in as usize, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }

    fn forget(&self, nodeid: u64, nlookup: u64) {
        let mut hiprio_queue = self.hiprio_queue.disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseForgetIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseForget as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let forgetin = FuseForgetIn { nlookup: nlookup };

        let headerin_bytes = headerin.as_bytes();
        let forgetin_bytes = forgetin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let concat_req = [headerin_bytes, forgetin_bytes, &headerout_buffer].concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = size_of::<FuseForgetIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        hiprio_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if hiprio_queue.should_notify() {
            hiprio_queue.notify();
        }
    }

    fn batch_forget(&self, forget_list: &[(u64, u64)]) {
        let mut hiprio_queue = self.hiprio_queue.disable_irq().lock();

        let headerin = FuseInHeader {
            len: (size_of::<FuseBatchForgetIn>() as u32 + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseBatchForget as u32,
            unique: 0,
            nodeid: 0,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let mut forgetin_bytes = Vec::new();
        for (nodeid, nlookup) in forget_list {
            let forgetin = FuseForgetOne {
                nodeid: *nodeid,
                nlookup: *nlookup,
            };
            forgetin_bytes.extend_from_slice(&forgetin.as_bytes());
        }

        let headerin_bytes = headerin.as_bytes();
        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let concat_req = [headerin_bytes, &forgetin_bytes, &headerout_buffer].concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = forget_list.len() * size_of::<FuseForgetOne>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        hiprio_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if hiprio_queue.should_notify() {
            hiprio_queue.notify();
        }
    }
    fn link(&self, nodeid: u64, oldnodeid: u64, name: Vec<u8>) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseLinkIn>() as u32
                + prepared_name.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseLink as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let linkin = FuseLinkIn {
            oldnodeid: oldnodeid,
        };

        let headerin_bytes = headerin.as_bytes();
        let linkin_bytes = linkin.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let linkout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            linkin_bytes,
            prepared_name_bytes,
            &headerout_buffer,
            &linkout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len() + size_of::<FuseLinkIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }
    fn unlink(&self, nodeid: u64, name: Vec<u8>) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();

        let prepared_name = fuse_pad_str(&String::from_utf8(name).unwrap(), true);

        let headerin = FuseInHeader {
            len: (size_of::<FuseUnlinkIn>() as u32
                + prepared_name.len() as u32
                + size_of::<FuseInHeader>() as u32),
            opcode: FuseOpcode::FuseUnlink as u32,
            unique: 0,
            nodeid: nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        };

        let headerin_bytes = headerin.as_bytes();
        let prepared_name_bytes = prepared_name.as_slice();

        let headerout_buffer = [0u8; size_of::<FuseOutHeader>()];
        let unlinkout_bytes = [0u8; size_of::<FuseEntryOut>()];
        let concat_req = [
            headerin_bytes,
            prepared_name_bytes,
            &headerout_buffer,
            &unlinkout_bytes,
        ]
        .concat();

        let mut reader = VmReader::from(concat_req.as_slice());
        let mut writer = self.request_buffers[0].writer().unwrap();
        let len = writer.write(&mut reader);
        let len_in = prepared_name.len() + size_of::<FuseUnlinkIn>() + size_of::<FuseInHeader>();

        self.request_buffers[0].sync(0..len).unwrap();
        let slice_in = DmaStreamSlice::new(&self.request_buffers[0], 0, len_in);
        let slice_out = DmaStreamSlice::new(&self.request_buffers[0], len_in, len);

        request_queue
            .add_dma_buf(&[&slice_in], &[&slice_out])
            .unwrap();

        if request_queue.should_notify() {
            request_queue.notify();
        }
    }
}

impl FilesystemDevice {
    /// Negotiate features for the device specified bits 0~23
    pub fn negotiate_features(features: u64) -> u64 {
        let device_features = FilesystemFeatures::from_bits_truncate(features);
        let supported_features = FilesystemFeatures::supported_features();
        let filesystem_features = device_features & supported_features;
        debug!("features negotiated: {:?}", filesystem_features);
        filesystem_features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioFilesystemConfig::new_manager(transport.as_ref());
        let fs_config: VirtioFilesystemConfig = config_manager.read_config();
        early_print!(
            "virtio_filesystem_config_notify_buf_size = {:?}\n",
            fs_config.notify_buf_size
        );
        early_print!(
            "virtio_filesystem_config_num_request_queues = {:?}\n",
            fs_config.num_request_queues
        );
        early_print!("virtio_filesystem_config_tag = {:?}\n", fs_config.tag);

        const HIPRIO_QUEUE_INDEX: u16 = 0;
        // const NOTIFICATION_QUEUE_INDEX: u16 = 1;
        const REQUEST_QUEUE_BASE_INDEX: u16 = 1;
        let hiprio_queue =
            SpinLock::new(VirtQueue::new(HIPRIO_QUEUE_INDEX, 2, transport.as_mut()).unwrap());
        // let notification_queue= SpinLock::new(VirtQueue::new(NOTIFICATION_QUEUE_INDEX, 2, transport.as_mut()).unwrap());
        let mut request_queues = Vec::new();
        for i in 0..fs_config.num_request_queues {
            request_queues.push(SpinLock::new(
                VirtQueue::new(REQUEST_QUEUE_BASE_INDEX + (i as u16), 4, transport.as_mut())
                    .unwrap(),
            ))
        }

        let hiprio_buffer = {
            let vm_segment = FrameAllocOptions::new().alloc_segment(3).unwrap();
            DmaStream::map(vm_segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };

        let mut request_buffers = Vec::new();
        for _ in 0..fs_config.num_request_queues {
            let request_buffer = {
                let vm_segment = FrameAllocOptions::new().alloc_segment(3).unwrap();
                DmaStream::map(vm_segment.into(), DmaDirection::Bidirectional, false).unwrap()
            };
            request_buffers.push(request_buffer);
        }

        let device = Arc::new(Self {
            config_manager: config_manager,
            transport: SpinLock::new(transport),
            hiprio_queue: hiprio_queue,
            // notification_queue: notification_queue,
            request_queues: request_queues,
            hiprio_buffer: hiprio_buffer,
            request_buffers: request_buffers,
        });
        let handle_request = {
            let device = device.clone();
            move |_: &TrapFrame| device.handle_recv_irq()
        };
        let config_space_change = |_: &TrapFrame| early_print!("Config Changed\n");
        let mut transport = device.transport.disable_irq().lock();
        transport
            .register_queue_callback(
                REQUEST_QUEUE_BASE_INDEX + 0,
                Box::new(handle_request),
                false,
            )
            .unwrap();
        transport
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();
        transport.finish_init();
        drop(transport);

        // device.init();
        test_device(&device);

        Ok(())
    }

    fn handle_recv_irq(&self) {
        let mut request_queue = self.request_queues[0].disable_irq().lock();
        let Ok((_, len)) = request_queue.pop_used() else {
            return;
        };
        self.request_buffers[0].sync(0..len as usize).unwrap();
        let mut reader = self.request_buffers[0].reader().unwrap();
        let headerin = reader.read_val::<FuseInHeader>().unwrap();

        match FuseOpcode::try_from(headerin.opcode).unwrap() {
            FuseOpcode::FuseInit => {
                let _datain = reader.read_val::<FuseInitIn>().unwrap();
                let _headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseInitOut>().unwrap();
                early_print!("Received Init Msg\n");
                early_print!("major:{:?}\n", dataout.major);
                early_print!("minor:{:?}\n", dataout.minor);
                early_print!("flags:{:?}\n", dataout.flags);
                early_println!();
            }
            FuseOpcode::FuseReaddir => {
                // 这里的datain千万不要注释，注释掉会出bug！！！！
                let _datain = reader.read_val::<FuseReadIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let readdir_out = FuseReaddirOut::read_dirent(&mut reader, headerout);

                early_print!(
                    "Readdir response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                for dirent_name in readdir_out.dirents {
                    let dirent = dirent_name.dirent;
                    let name = String::from_utf8(dirent_name.name).unwrap();
                    early_print!("Readdir response received: inode={:?}, off={:?}, namelen={:?}, type:{:?}, filename={:?}\n", 
                        dirent.ino, dirent.off, dirent.namelen, dirent.type_, name);
                }
                early_println!();
            }
            FuseOpcode::FuseOpendir => {
                let _datain = reader.read_val::<FuseOpenIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseOpenOut>().unwrap();
                early_print!(
                    "Readdir response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("fh:{:?}\n", dataout.fh);
                early_print!("open_flags:{:?}\n", dataout.open_flags);
                early_print!("backing_id:{:?}\n", dataout.backing_id);
                early_println!();
            }
            FuseOpcode::FuseOpen => {
                let _datain = reader.read_val::<FuseOpenIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseOpenOut>().unwrap();
                early_print!(
                    "Open response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("fh:{:?}\n", dataout.fh);
                early_print!("open_flags:{:?}\n", dataout.open_flags);
                early_print!("backing_id:{:?}\n", dataout.backing_id);
            }
            FuseOpcode::FuseRead => {
                let _datain = reader.read_val::<FuseReadIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                //The requested action is to read up to size bytes of the file or directory, starting at offset. The bytes should be returned directly following the usual reply header.
                // let dataout = reader.read_val::<Vec<u8>>().unwrap();
                early_print!(
                    "Read response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                // early_println!();
                // if the file is not empty
                if headerout.len > size_of::<FuseOutHeader>() as u32 {
                    let data_len = headerout.len - size_of::<FuseOutHeader>() as u32;
                    let mut dataout_buf = vec![0u8; data_len as usize];
                    let mut writer = VmWriter::from(dataout_buf.as_mut_slice());
                    writer.write(&mut reader);
                    let data_utf8 = String::from_utf8(dataout_buf).unwrap();
                    early_print!("Read response received: data={:?}\n", data_utf8);
                }
                // early_print!("Read data: {:?}", dataout);
            }
            FuseOpcode::FuseFlush => {
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "Flush response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
            }
            FuseOpcode::FuseReleasedir => {
                let _datain = reader.read_val::<FuseReleaseIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                // let dataout = reader.read_val::<FuseReleaseOut>().unwrap();
                early_print!(
                    "Releasedir response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
                // early_print!("fh:{:?}\n", dataout.fh);
            }
            FuseOpcode::FuseGetattr => {
                let _datain = reader.read_val::<FuseGetattrIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseAttrOut>().unwrap();
                early_print!(
                    "Getattr response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseSetattr => {
                let _datain = reader.read_val::<FuseSetattrIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseAttrOut>().unwrap();
                early_print!(
                    "Setattr response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseLookup => {
                let _name = reader.read_val::<FuseInHeader>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Lookup response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!("test for lookup");
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseRelease => {
                let _datain = reader.read_val::<FuseReleaseIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                // let dataout = reader.read_val::<FuseReleaseOut>().unwrap();
                early_print!(
                    "Release response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
                // early_print!("fh:{:?}\n", dataout.fh);
            }
            FuseOpcode::FuseWrite => {
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "Write response received: len={:?}, error={:?}\n",
                    headerout.len,
                    headerout.error
                );
                if headerout.len > size_of::<FuseOutHeader>() as u32 {
                    let writeout = reader.read_val::<FuseWriteOut>().unwrap();
                    early_print!("Write response received: size={:?}\n", writeout.size);
                }
            }
            FuseOpcode::FuseAccess => {
                let _datain = reader.read_val::<FuseAccessIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseAttrOut>().unwrap();
                early_print!(
                    "Access response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseStatfs => {
                let _datain = reader.read_val::<FuseInHeader>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseStatfsOut>().unwrap();
                early_print!(
                    "Statfs response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("blocks:{:?}\n", dataout.st.blocks);
                early_print!("bfree:{:?}\n", dataout.st.bfree);
                early_print!("bavail:{:?}\n", dataout.st.bavail);
                early_print!("files:{:?}\n", dataout.st.files);
                early_print!("ffree:{:?}\n", dataout.st.ffree);
                early_print!("bsize:{:?}\n", dataout.st.bsize);
                early_print!("namelen:{:?}\n", dataout.st.namelen);
                early_print!("frsize:{:?}\n", dataout.st.frsize);
                early_print!("padding:{:?}\n", dataout.st.padding);
                early_print!("spare:{:?}\n", dataout.st.spare);

                early_println!();
            }
            FuseOpcode::FuseInterrupt => {
                let _datain = reader.read_val::<FuseInterruptIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "Interrupt response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
            }
            FuseOpcode::FuseMkdir => {
                let _datain = reader.read_val::<FuseMkdirIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Mkdir response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseCreate => {
                let _datain = reader.read_val::<FuseCreateIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Create response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseDestroy => {
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "Destroy response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
            }
            FuseOpcode::FuseRename => {
                let _datain = reader.read_val::<FuseRenameIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Rename response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseRename2 => {
                let _datain = reader.read_val::<FuseRename2In>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Rename2 response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseForget => {
                let _datain = reader.read_val::<FuseForgetIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "Forget response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
            }
            FuseOpcode::FuseBatchForget => {
                let _datain = reader.read_val::<FuseBatchForgetIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                early_print!(
                    "BatchForget response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_println!();
            }
            FuseOpcode::FuseLink => {
                let _datain = reader.read_val::<FuseLinkIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Link response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            FuseOpcode::FuseUnlink => {
                let _datain = reader.read_val::<FuseUnlinkIn>().unwrap();
                let headerout = reader.read_val::<FuseOutHeader>().unwrap();
                let dataout = reader.read_val::<FuseEntryOut>().unwrap();
                early_print!(
                    "Unlink response received: len = {:?}, error = {:?}\n",
                    headerout.len,
                    headerout.error
                );
                early_print!("nodeid:{:?}\n", dataout.nodeid);
                early_print!("generation:{:?}\n", dataout.generation);
                early_print!("entry_valid:{:?}\n", dataout.entry_valid);
                early_print!("attr_valid:{:?}\n", dataout.attr_valid);
                early_print!("entry_valid_nsec:{:?}\n", dataout.entry_valid_nsec);
                early_print!("attr_valid_nsec:{:?}\n", dataout.attr_valid_nsec);
                early_print!("attr:{:?}\n", dataout.attr);
                early_println!();
            }
            _ => {}
        }
        drop(request_queue);
        test_device(&self);
    }
}

static TEST_COUNTER: RwLock<u32> = RwLock::new(0);
pub fn test_device(device: &FilesystemDevice) {
    let mut test_counter = TEST_COUNTER.write();
    *test_counter += 1;
    drop(test_counter);
    let test_counter = TEST_COUNTER.read();
    match *test_counter {
        // // test lookup
        // 0 => device.lookup(1, Vec::from("testf01")),

        // // test read
        // 1 => device.lookup(1, Vec::from("testf01")),
        // 2 => device.open(2, 0),
        // 3 => device.read(2, 0, 0, 128),
        // 4 => device.lookup(1, Vec::from("testf02")),
        // 5 => device.open(3, 0),
        // 6 => device.read(3, 1, 0, 128),

        // // test write
        // 1 => device.lookup(1, Vec::from("testf03")),
        // 2 => device.open(2, 2),
        // 3 => device.write(2, 0, 0, "Hello world 123".as_bytes()),

        // // test mkdir
        // 1 => device.lookup(1, "testdir".as_bytes().to_vec()),
        // 2 => device.mkdir(2, 0o755, 0o777, "testdir2".as_bytes().to_vec()),

        // // test create
        // 1 => device.lookup(1, "testdir".as_bytes().to_vec()),
        // 2 => device.create(2, "test_create".as_bytes().to_vec(), 0o755, 0o777, 2),
        _ => (),
    };
}
