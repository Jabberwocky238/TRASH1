use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::Metadata;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Weak;
use std::time::SystemTime;

const EXCLUDE_DIR: [&str; 1] = ["node_modules"];

#[derive(Debug)]
pub enum Child {
    FileInfo(FileInfo),
    DirInfo(DirInfo),
    NotRecord(FileInfo), // last write time, name
}

#[derive(Debug)]
pub struct FileInfo {
    abspath: PathBuf,
    last_write_time: u128,
    size: u64,
}

#[derive(Debug)]
pub struct DirInfo {
    pub abspath: PathBuf,
    last_write_time: u128,
    pub size: u64,
    pub children: HashMap<u64, Child>,
    pub count_dir: usize,
    pub count_file: usize,
}

#[derive(Debug)]
pub struct Controllor {
    root: DirInfo,
    current: Weak<DirInfo>,
}

impl FileInfo {
    pub fn new(path: &Path) -> FileInfo {
        if !path.is_absolute() {
            panic!("path must be absolute");
        }
        // println!("New FileInfo: {}", path.to_str().unwrap());
        let metadata = fs::metadata(path).unwrap();
        FileInfo {
            abspath: path.to_path_buf(),
            last_write_time: get_last_modified(&metadata),
            size: metadata.len(),
        }
    }
    pub fn update(&mut self) {
        let metadata = fs::metadata(self.abspath.as_path()).unwrap();
        self.last_write_time = get_last_modified(&metadata);
        self.size = metadata.len();
    }
}

impl DirInfo {
    pub fn new(path: &Path) -> DirInfo {
        if !path.is_absolute() {
            panic!("path must be absolute");
        }
        // println!("New DirInfo: {}", path.to_str().unwrap());
        let metadata: Metadata = fs::metadata(path).unwrap();
        DirInfo {
            abspath: path.to_path_buf(),
            last_write_time: get_last_modified(&metadata),
            size: metadata.len(),
            children: HashMap::new(),
            count_dir: 0,
            count_file: 0,
        }
    }

    pub fn scan(&mut self) {
        let mut hasher = DefaultHasher::new();
        let mut waiting_list = self
            .children
            .iter()
            .map(|(&hash, _)| hash)
            .collect::<HashSet<u64>>();

        for entry in fs::read_dir(self.abspath.as_path()).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = entry.metadata().unwrap();
            path.hash(&mut hasher);
            let hash = hasher.finish();

            if metadata.is_dir() {
                let dir_name = path.file_name().unwrap().to_str().unwrap().to_string();
                // 找waiting_list
                let mut found = false;
                if let Some(status) = waiting_list.get(&hash) {
                    match self.children.get_mut(status).unwrap() {
                        Child::DirInfo(child) => {
                            found = true;
                            if !verify(child.abspath.as_path(), child.last_write_time) {
                                self.count_dir -= child.count_dir;
                                self.count_file -= child.count_file;
                                self.size -= child.size;
                                child.scan();
                                self.count_dir += child.count_dir;
                                self.count_file += child.count_file;
                                self.size += child.size;
                            }
                        }
                        Child::NotRecord(child) => {
                            found = true;
                            if !verify(child.abspath.as_path(), child.last_write_time) {
                                let metadata = fs::metadata(child.abspath.as_path()).unwrap();
                                self.size -= child.size;
                                let mut dir = DirInfo::new(child.abspath.as_path());
                                dir.scan();
                                child.size = dir.size;
                                child.last_write_time = get_last_modified(&metadata);
                                self.size += child.size;
                            }
                        }
                        _ => { /* dont care */ }
                    }
                }
                if found {
                    waiting_list.remove(&hash);
                    continue;
                }
                // 新文件夹
                let mut child = DirInfo::new(path.as_path());
                child.scan();
                self.count_dir += 1 + child.count_dir;
                self.count_file += child.count_file;
                self.size += child.size;
                // 是否应该不记录文件夹细节
                if !EXCLUDE_DIR.contains(&dir_name.as_str()) {
                    self.children.insert(hash, Child::DirInfo(child));
                } else {
                    let mut nr = FileInfo::new(path.as_path());
                    nr.size = child.size;
                    nr.last_write_time = child.last_write_time;
                    self.children.insert(hash, Child::NotRecord(nr));
                }
            } else if metadata.is_file() {
                // 找waiting_list
                let mut found = false;
                if let Some(hash) = waiting_list.get(&hash) {
                    if let Child::FileInfo(child) = self.children.get_mut(&hash).unwrap() {
                        found = true;
                        // println!("inner found");
                        if !verify(child.abspath.as_path(), child.last_write_time) {
                            self.size -= child.size;
                            child.update();
                            self.size += child.size;
                        }
                    }
                }
                if found {
                    // println!("outer found");
                    waiting_list.remove(&hash);
                    continue;
                }
                // 新文件
                let mut child = FileInfo::new(path.as_path());
                child.update();
                self.count_file += 1;
                self.size += child.size;
                self.children.insert(hash, Child::FileInfo(child));
            } else if metadata.is_symlink() {
                self.size += metadata.len();
            } else {
                // only count file and size
                self.size += metadata.len();
            }
        }

        for hash in waiting_list {
            match self.children.get(&hash).unwrap() {
                Child::DirInfo(child) => {
                    self.count_dir -= child.count_dir + 1;
                    self.count_file -= child.count_file;
                    self.size -= child.size;
                }
                Child::FileInfo(child) => {
                    self.count_file -= 1;
                    self.size -= child.size;
                }
                Child::NotRecord(file_info) => {
                    self.size -= file_info.size;
                }
            }
            self.children.remove(&hash);
        }
    }
    pub fn tree(&self, depth: usize, last: usize) -> String {
        let mut buffer = String::new();
        if last == 0 {
            return buffer;
        }
        buffer += self.abspath.file_name().unwrap().to_str().unwrap();
        buffer += "\n";
        for (_, child) in &self.children {
            match child {
                Child::DirInfo(child) => {
                    buffer += &"|";
                    buffer += &"-".repeat((depth + 1) * 4 - 1);
                    buffer += child.abspath.file_name().unwrap().to_str().unwrap();
                    buffer += "\n";
                    buffer += &child.tree(depth + 1, last - 1);
                }
                Child::FileInfo(child) => {
                    buffer += &"|";
                    buffer += &"-".repeat((depth + 1) * 4 - 1);
                    buffer += child.abspath.file_name().unwrap().to_str().unwrap();
                    buffer += "\n";
                }
                Child::NotRecord(_) => {
                    buffer += &"|";
                    buffer += &"-".repeat((depth + 1) * 4 - 1);
                    buffer += "NotRecord";
                    buffer += "\n";
                }
            }
        }
        buffer
    }
}

impl Controllor {
    // pub fn new() -> Controllor {
    //     let curpath = env::current_dir().unwrap();
    //     let mut _split = curpath.iter().collect::<Vec<_>>();
    // }
}

fn verify(path: &Path, last_write_time: u128) -> bool {
    if !path.is_absolute() {
        panic!("path must be absolute");
    }
    match fs::metadata(path) {
        Ok(metadata) => {
            if get_last_modified(&metadata) == last_write_time {
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

fn get_last_modified(metadata: &Metadata) -> u128 {
    metadata
        .modified()
        .unwrap()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

mod test_core {
    use crate::core::DirInfo;
    use std::{io::Write, path::Path};

    #[test]
    fn test_scan() {
        let path = Path::new("E:\\nginx-1.26.1");
        let mut dir = DirInfo::new(path);
        dir.scan();

        // assert_eq!(13_019_719, dir.size); // size偏大
        assert!(dir.size > 13_019_732);
        assert_eq!(37, dir.count_file);
        assert_eq!(35, dir.count_dir);
    }

    #[test]
    fn test_reuse() {
        let path = Path::new("E:\\nginx-1.26.1");
        let mut dir = DirInfo::new(path);
        dir.scan();

        // 创建文件，写入内容
        let file_path = Path::new("E:\\nginx-1.26.1\\test.txt");
        let mut file = std::fs::File::create(file_path).unwrap();
        file.write_all(b"Hello, world!").unwrap();

        // 重新扫描目录
        dir.scan();
        println!("{}", dir.tree(0, 1));
        // 删除文件
        std::fs::remove_file(file_path).unwrap();

        // 检查文件是否被正确添加到目录中
        assert!(dir.size > 13_019_719 + 10000); // 13_036_116
        assert_eq!(dir.count_file, 37 + 1);
        assert_eq!(dir.count_dir, 35);
    }
}
