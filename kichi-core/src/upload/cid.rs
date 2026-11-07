//! CID / GCID 计算：小文件全量 SHA-1、大文件三段采样 + 分块 SHA-1 回退。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha1::{Digest as Sha1Digest, Sha1};
use sha2::{Digest as Sha256Digest, Sha256};

use crate::Error;

use super::io::{hex_lower, hex_upper, FileRangeReader};
use super::util::ensure_not_cancelled;
use super::{UploadProgress, UploadStage, CID_FULL_FILE_LIMIT, CID_SAMPLE_BYTES};

pub(super) fn cid_for_file<C, P>(
    path: &Path,
    size: u64,
    cancelled: &C,
    progress: &mut P,
) -> Result<String, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    let mut file =
        File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
    let mut hasher = Sha1::new();
    let mut hashed = 0u64;
    if size <= CID_FULL_FILE_LIMIT {
        hash_range(
            &mut file,
            0,
            size,
            &mut hasher,
            cancelled,
            progress,
            &mut hashed,
            size,
        )?;
    } else {
        let total = CID_SAMPLE_BYTES * 3;
        for start in [0, size / 3, size - CID_SAMPLE_BYTES] {
            hash_range(
                &mut file,
                start,
                CID_SAMPLE_BYTES,
                &mut hasher,
                cancelled,
                progress,
                &mut hashed,
                total,
            )?;
        }
    }
    Ok(hex_upper(&hasher.finalize()))
}

pub(super) fn full_gcid_for_file<C, P>(
    path: &Path,
    size: u64,
    cancelled: &C,
    progress: &mut P,
) -> Result<String, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    ensure_not_cancelled(cancelled)?;
    progress(UploadProgress {
        stage: UploadStage::Hashing,
        bytes: 0,
        completed_bytes: None,
        total: size,
    });
    let mut file =
        File::open(path).map_err(|err| Error::Other(format!("无法打开本地文件：{err}")))?;
    let block_size = gcid_block_size(size);
    let mut total = Sha1::new();
    let mut read_total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    while read_total < size {
        ensure_not_cancelled(cancelled)?;
        let target = block_size.min(size - read_total);
        let mut block = Sha1::new();
        let mut block_read = 0u64;
        while block_read < target {
            ensure_not_cancelled(cancelled)?;
            let want =
                usize::try_from((target - block_read).min(buf.len() as u64)).unwrap_or(buf.len());
            let got = file
                .read(&mut buf[..want])
                .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
            if got == 0 {
                return Err(Error::Other("文件在上传准备期间被截短".to_owned()));
            }
            block.update(&buf[..got]);
            block_read += got as u64;
            read_total += got as u64;
            progress(UploadProgress {
                stage: UploadStage::Hashing,
                bytes: read_total,
                completed_bytes: None,
                total: size,
            });
        }
        total.update(block.finalize());
    }
    if read_total != size {
        return Err(Error::Other("文件大小在上传准备期间发生变化".to_owned()));
    }
    Ok(hex_lower(&total.finalize()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn hash_range<C, P>(
    file: &mut File,
    start: u64,
    length: u64,
    hasher: &mut Sha1,
    cancelled: &C,
    progress: &mut P,
    hashed: &mut u64,
    total: u64,
) -> Result<(), Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    file.seek(SeekFrom::Start(start))
        .map_err(|err| Error::Other(format!("无法定位本地文件：{err}")))?;
    let mut left = length;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        ensure_not_cancelled(cancelled)?;
        let want = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
        let got = file
            .read(&mut buf[..want])
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if got == 0 {
            return Err(Error::Other("文件在上传准备期间被截短".to_owned()));
        }
        hasher.update(&buf[..got]);
        left -= got as u64;
        *hashed += got as u64;
        progress(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: *hashed,
            completed_bytes: None,
            total,
        });
    }
    Ok(())
}

pub(super) fn sha256_file_range<C>(
    path: &Path,
    offset: u64,
    length: u64,
    cancelled: &C,
) -> Result<Vec<u8>, Error>
where
    C: Fn() -> bool,
{
    let mut reader = FileRangeReader::open(path, offset, length)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut remaining = length;
    while remaining > 0 {
        ensure_not_cancelled(cancelled)?;
        let limit = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = reader
            .read(&mut buffer[..limit])
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if read == 0 {
            return Err(Error::Other("文件在上传期间被截短".to_owned()));
        }
        hasher.update(&buffer[..read]);
        remaining = remaining.saturating_sub(read as u64);
    }
    Ok(hasher.finalize().to_vec())
}

pub(super) fn sha256_hex<C, P>(
    file: &mut File,
    cancelled: &C,
    progress: &mut P,
    total: u64,
) -> Result<Vec<u8>, Error>
where
    C: Fn() -> bool,
    P: FnMut(UploadProgress),
{
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut read = 0u64;
    loop {
        ensure_not_cancelled(cancelled)?;
        let got = file
            .read(&mut buf)
            .map_err(|err| Error::Other(format!("读取本地文件失败：{err}")))?;
        if got == 0 {
            break;
        }
        hasher.update(&buf[..got]);
        read += got as u64;
        progress(UploadProgress {
            stage: UploadStage::Hashing,
            bytes: read,
            completed_bytes: None,
            total,
        });
    }
    if read != total {
        return Err(Error::Other("文件大小在上传准备期间发生变化".to_owned()));
    }
    Ok(hasher.finalize().to_vec())
}

pub(super) fn gcid_block_size(size: u64) -> u64 {
    let mut block: u64 = 0x40000;
    while size > block.saturating_mul(0x200) && block < 0x200000 {
        block <<= 1;
    }
    block
}
