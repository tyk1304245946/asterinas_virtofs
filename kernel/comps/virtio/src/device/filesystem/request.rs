// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};
use core::fmt::Debug;

use ostd::{
    early_print,
    mm::{VmReader, VmWriter},
    Pod,
};

use super::fuse::*;

pub trait AnyFuseDevice {
    // Send Init Request to Device.
    fn init(&self);
    fn readdir(&self, nodeid: u64, fh: u64, offset: u64, size: u32);
    fn opendir(&self, nodeid: u64, flags: u32);
    fn open(&self, nodeid: u64, flags: u32);
}

#[derive(Debug)]
#[repr(C)]
pub struct VirtioFsReq {
    //Device readable
    pub headerin: FuseInHeader,
    pub datain: Vec<u8>,

    //Device writable
    pub headerout: FuseOutHeader,
    pub dataout: Vec<u8>,
}

impl VirtioFsReq {
    pub fn into_bytes(&self) -> Vec<u8> {
        let fuse_in_header = self.headerin.as_bytes();
        let datain = self.datain.as_slice();
        let fuse_out_header = self.headerout.as_bytes();
        let dataout = self.dataout.as_slice();

        let total_len = fuse_in_header.len() + datain.len() + fuse_out_header.len() + dataout.len();

        let mut concat_req = vec![0u8; total_len];
        concat_req[0..fuse_in_header.len()].copy_from_slice(fuse_in_header);
        concat_req[fuse_in_header.len()..(fuse_in_header.len() + datain.len())]
            .copy_from_slice(datain);

        concat_req
    }
}

///FuseDirent with the file name
pub struct FuseDirentWithName {
    pub dirent: FuseDirent,
    pub name: Vec<u8>,
}
///Contain all directory entries for one directory
pub struct FuseReaddirOut {
    pub dirents: Vec<FuseDirentWithName>,
}
impl FuseReaddirOut {
    /// Read all directory entries from the buffer
    pub fn read_dirent(
        reader: &mut VmReader<'_, ostd::mm::Infallible>,
        out_header: FuseOutHeader,
    ) -> FuseReaddirOut {
        let mut len = out_header.len as i32 - size_of::<FuseOutHeader>() as i32;
        let mut dirents: Vec<FuseDirentWithName> = Vec::new();
        // For paddings between dirents
        let mut padding: Vec<u8> = vec![0 as u8; 8];
        while len > 0 {
            let dirent = reader.read_val::<FuseDirent>().unwrap();
            let mut file_name: Vec<u8>;

            file_name = vec![0 as u8; dirent.namelen as usize];
            let mut writer = VmWriter::from(file_name.as_mut_slice());
            writer.write(reader);
            let pad_len = (8 - (dirent.namelen & 0x7)) & 0x7; // pad to multiple of 8 bytes
            let mut pad_writer = VmWriter::from(&mut padding[0..pad_len as usize]);
            pad_writer.write(reader);
            dirents.push(FuseDirentWithName {
                dirent: dirent,
                name: file_name,
            });
            early_print!(
                "len: {:?} ,dirlen: {:?}, name_len: {:?}\n",
                len,
                size_of::<FuseDirent>() as u32 + dirent.namelen,
                dirent.namelen
            );
            len -= size_of::<FuseDirent>() as i32 + dirent.namelen as i32 + pad_len as i32;
        }
        FuseReaddirOut { dirents: dirents }
    }
}