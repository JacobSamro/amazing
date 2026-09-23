use crate::{Error, FrameMetadata};
use std::process::{Child, Command};

pub struct FrameHandle;
pub struct FrameView;
pub struct HandleChannel;
impl FrameHandle {
    pub fn new(_: u64, _: u32, _: u32, _: &[u8]) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
    pub fn metadata(&self) -> FrameMetadata {
        unreachable!()
    }
    pub fn local_view(&self) -> Result<FrameView, Error> {
        Err(Error::Unsupported)
    }
    pub fn publish(&self, _: &HandleChannel) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
}
impl FrameView {
    pub fn metadata(&self) -> FrameMetadata {
        unreachable!()
    }
    pub fn bytes(&self) -> &[u8] {
        unreachable!()
    }
}
impl HandleChannel {
    pub fn nonce_hex(&self) -> String {
        String::new()
    }
    pub fn prepare_child(_: &mut Command) -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
    pub fn from_child_env() -> Result<Self, Error> {
        Err(Error::Unsupported)
    }
    pub fn attach_child(&mut self, _: &Child) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    pub fn receive(&self, _: FrameMetadata, _: u32) -> Result<FrameView, Error> {
        Err(Error::Unsupported)
    }
}
