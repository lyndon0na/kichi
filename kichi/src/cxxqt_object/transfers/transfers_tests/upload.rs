use super::super::*;
use super::*;

#[test]
fn upload_record_maps_stage_target_and_final_name() {
    let mut record = row_from(
        8,
        0,
        "upload:/home/me/电影.mkv",
        &upload_job("电影.mkv", 100),
        1_790_000_000,
        1,
    );
    assert!(begin_record(&mut record, Instant::now()));

    for (stage, state_text, detail) in [
        (UploadStage::Preparing, "准备上传", "正在准备上传…"),
        (UploadStage::Hashing, "计算校验", "正在计算文件校验值…"),
        (UploadStage::Uploading, "上传中", "正在上传到 动画"),
        (
            UploadStage::Finalizing,
            "服务器处理",
            "上传完成，等待服务器处理…",
        ),
    ] {
        record.upload_progress = Some(UploadProgressState {
            stage,
            bytes: 50,
            completed_bytes: Some(50),
            total: 100,
        });
        record.bytes = 50;
        let map = record_map(&record, &[]);
        assert_eq!(text_of(&map, "direction"), "upload");
        assert_eq!(text_of(&map, "stateText"), state_text);
        assert_eq!(text_of(&map, "detail"), detail);
        assert_eq!(text_of(&map, "sizeText"), "50 B / 100 B");
        assert_eq!(text_of(&map, "path"), "");
    }

    record.state = TransferState::Done;
    record.job.name = "电影 (1).mkv".to_owned();
    record.upload_progress = Some(UploadProgressState {
        stage: UploadStage::Finalizing,
        bytes: 100,
        completed_bytes: Some(100),
        total: 100,
    });
    let map = record_map(&record, &[]);
    assert_eq!(text_of(&map, "name"), "电影 (1).mkv");
    assert_eq!(text_of(&map, "detail"), "已上传到 动画");
    assert_eq!(text_of(&map, "path"), "", "上传成功没有本地位置可打开");
}

#[test]
fn upload_live_bytes_stay_separate_from_resumable_bytes() {
    const MIB: u64 = 1024 * 1024;
    let mut records = Vec::new();
    records.push(row_from(
        78,
        0,
        "upload:/home/me/big.mkv",
        &upload_job("big.mkv", 10 * MIB),
        1_790_000_000,
        1,
    ));
    begin_in(&mut records, 78, 0);

    upload_progress_in(
        &mut records,
        78,
        0,
        UploadProgressState {
            stage: UploadStage::Uploading,
            bytes: 7 * MIB,
            completed_bytes: Some(5 * MIB),
            total: 10 * MIB,
        },
    );
    let child = find_file(&mut records, 78, 0).expect("上传行存在");
    assert_eq!(child.bytes, 5 * MIB, "历史与断点只认完成分片");
    assert_eq!(
        child.upload_progress.map(|progress| progress.bytes),
        Some(7 * MIB),
        "界面进度包含在途字节"
    );
    assert_eq!(percent_of(child), Some(70));

    // 分片重试会让显示进度回退，但不能把断点进度一起改掉。
    upload_progress_in(
        &mut records,
        78,
        0,
        UploadProgressState {
            stage: UploadStage::Uploading,
            bytes: 6 * MIB,
            completed_bytes: Some(5 * MIB),
            total: 10 * MIB,
        },
    );
    let child = find_file(&mut records, 78, 0).expect("上传行存在");
    assert_eq!(child.bytes, 5 * MIB);
    assert_eq!(
        child.upload_progress.map(|progress| progress.bytes),
        Some(6 * MIB)
    );

    // 校验阶段没有“完成分片”口径，不能把之前的断点进度清成 0。
    upload_progress_in(
        &mut records,
        78,
        0,
        UploadProgressState {
            stage: UploadStage::Hashing,
            bytes: 20 * 1024,
            completed_bytes: None,
            total: 10 * MIB,
        },
    );
    let child = find_file(&mut records, 78, 0).expect("上传行存在");
    assert_eq!(child.bytes, 5 * MIB);
}

#[test]
fn folder_upload_progress_uses_live_bytes() {
    const MIB: u64 = 1024 * 1024;
    let mut records = Vec::new();
    folder_batch(&mut records, 79, "合集", &[("a.mkv", 10), ("b.mkv", 20)]);
    let child = child_mut(&mut records, 79, 0);
    child.state = TransferState::Running;
    child.job.direction = TransferDirection::Upload;
    child.job.upload = Some(UploadJob {
        local_path: PathBuf::from("/home/me/a.mkv"),
        ..UploadJob::default()
    });
    child.bytes = 5 * MIB;
    child.upload_progress = Some(UploadProgressState {
        stage: UploadStage::Uploading,
        bytes: 7 * MIB,
        completed_bytes: Some(5 * MIB),
        total: 10 * MIB,
    });
    assert_eq!(display_bytes(child), 7 * MIB);

    child.upload_progress = Some(UploadProgressState {
        stage: UploadStage::Hashing,
        bytes: 20 * 1024,
        completed_bytes: None,
        total: 10 * MIB,
    });
    assert_eq!(display_bytes(child), 5 * MIB);
}

#[test]
fn upload_resume_targets_cover_single_and_folder_paths() {
    let single = row_from(
        91,
        0,
        "upload:/home/me/single.mkv",
        &upload_job("single.mkv", 10),
        1_790_000_000,
        1,
    );
    assert_eq!(
        upload_resume_targets_in(&[single], 91, None),
        vec![(
            PathBuf::from("/home/me/single.mkv"),
            "cloud-folder".to_owned()
        )]
    );

    let mut root_job = upload_job("root.mkv", 10);
    let root_upload = root_job.upload.as_mut().expect("root upload");
    root_upload.parent_id.clear();
    let root = row_from(
        93,
        0,
        "upload:/home/me/root.mkv",
        &root_job,
        1_790_000_000,
        1,
    );
    assert_eq!(
        upload_resume_targets_in(&[root], 93, None),
        vec![(PathBuf::from("/home/me/root.mkv"), String::new())],
        "上传到根目录时 parent_id 为空串，仍必须清理对应断点"
    );

    let mut parent = upload_job("剧集", 0);
    parent.kind = EntryKind::Folder;
    parent.upload.as_mut().expect("parent upload").local_path = PathBuf::from("/home/me/剧集");
    let mut parent_row = row_from(
        92,
        0,
        "upload-folder:/home/me/剧集",
        &parent,
        1_790_000_000,
        1,
    );
    parent_row.upload_dirs = vec![UploadDir {
        path: vec!["剧集".to_owned(), "子目录".to_owned()],
        id: "cloud-subdir".to_owned(),
    }];

    let mut child_job = upload_job("child.mkv", 10);
    child_job.sub_dir = vec!["剧集".to_owned(), "子目录".to_owned()];
    child_job.upload.as_mut().expect("child upload").local_path =
        PathBuf::from("/home/me/剧集/子目录/child.mkv");
    child_job
        .upload
        .as_mut()
        .expect("child upload")
        .cloud_parent_id = "cloud-subdir-renamed".to_owned();
    let mut child = row_from(92, 0, child_job.id.as_str(), &child_job, 1_790_000_000, 1);
    child.child = true;
    child.state = TransferState::Queued;

    let mut records = vec![parent_row, child];
    let targets = upload_resume_targets_in(&records, 92, None);
    assert_eq!(
        targets,
        vec![
            (
                PathBuf::from("/home/me/剧集/子目录/child.mkv"),
                "cloud-folder".to_owned()
            ),
            (
                PathBuf::from("/home/me/剧集/子目录/child.mkv"),
                "cloud-subdir-renamed".to_owned()
            ),
            (
                PathBuf::from("/home/me/剧集/子目录/child.mkv"),
                "cloud-subdir".to_owned()
            ),
        ],
        "文件夹父行不产生断点；子文件同时覆盖目标目录、实际父目录和目录树 id"
    );

    records[1].state = TransferState::Running;
    assert!(
        upload_resume_targets_in(&records, 92, None).is_empty(),
        "已开始发送的文件必须由 worker 自己清理，避免误删刚成功的上传"
    );
}
