use nix::{mount::MsFlags, sched::CloneFlags};
use std::path::{Path, PathBuf};

use crate::{
    child_process::{error::Result, syscall},
    IDMap, Mount, Namespaces,
};

pub fn init(
    namespaces: &Namespaces,
    _uid_mappings: &IDMap,
    _gid_mappings: &IDMap,
    hostname: &str,
    rootfs: &Path,
    mounts: &[Mount],
    work_dir: &Path,
) -> Result<()> {
    let clone_flags = namespaces.to_clone_flags();
    syscall::unshare(clone_flags)?;

    if clone_flags.contains(CloneFlags::CLONE_NEWNS) {
        init_mount_namespace(rootfs, mounts, work_dir)?;
    }
    if clone_flags.contains(CloneFlags::CLONE_NEWUTS) {
        init_uts_namespace(hostname)?;
    }

    Ok(())
}

// [pivot_root]: https://man7.org/linux/man-pages/man2/pivot_root.2.html
fn init_mount_namespace(new_root: &Path, mounts: &[Mount], work_dir: &Path) -> Result<()> {
    // Ensure that 'new_root' and its parent mount don't have
    // shared propagation (which would cause pivot_root() to
    // return an error), and prevent propagation of mount
    // events to the initial mount namespace.
    syscall::mount_root()?;

    // Ensure that 'new_root' is a mount point.
    syscall::mount(new_root, new_root, MsFlags::MS_BIND)?;
    syscall::chdir(new_root)?;

    // Mount rootfs.
    {
        // Mount file system.
        for mount in mounts {
            let metadata = syscall::metadata(&mount.host_path)?;
            let target = &mount
                .container_path
                .strip_prefix("/")
                .unwrap_or(&mount.container_path);
            match metadata.is_dir() {
                true => syscall::mkdir(target)?,
                _ => syscall::touch(target)?,
            }
            syscall::mount(&mount.host_path, target, MsFlags::MS_BIND)?;
        }

        // Mount devfs.
        syscall::mkdir(new_root.join("dev"))?;
        for host_path in ["/dev/null", "/dev/random", "/dev/urandom", "/dev/zero"] {
            let target = host_path.strip_prefix('/').unwrap_or(host_path);
            syscall::mknod(&PathBuf::from(target))?;
            syscall::mount(host_path, target, MsFlags::MS_BIND)?;
        }

        // Hang on to the old proc in order to mount the new proc later on.
        let target = new_root.join(Mount::PUT_OLD_PROC_DIR.0);
        syscall::mkdir(&target)?;
        syscall::mount("/proc", &target, MsFlags::MS_BIND | MsFlags::MS_REC)?;
        syscall::mkdir(new_root.join(Mount::PROC_DIR.0))?;

        // Mount WORK_DIR.
        let target = new_root.join(Mount::WORK_DIR.0);
        syscall::mkdir(&target)?;
        syscall::mount(work_dir, &target, MsFlags::MS_BIND)?;
    }

    // Create directory to which old root will be pivoted.
    syscall::mkdir(Mount::PUT_OLD_DIR.0)?;

    // Pivot the root filesystem.
    syscall::pivot_root(".", Mount::PUT_OLD_DIR.0)?;
    syscall::chdir("/")?;

    // Unmount old root and remove mount point.
    syscall::unmount(Mount::PUT_OLD_DIR.1)?;
    syscall::rmdir(Mount::PUT_OLD_DIR.1)
}

fn init_uts_namespace(hostname: &str) -> Result<()> {
    syscall::sethostname(hostname)
}

pub fn reinit(
    namespaces: &Namespaces,
    uid_mappings: &IDMap,
    gid_mappings: &IDMap,
    mounts: &[Mount],
) -> Result<()> {
    let clone_flags = namespaces.to_clone_flags();

    if clone_flags.contains(CloneFlags::CLONE_NEWNS) {
        reinit_mount_namespace(mounts)?;
    }
    if clone_flags.contains(CloneFlags::CLONE_NEWUSER) {
        reinit_user_namespace(uid_mappings, gid_mappings)?;
    }

    Ok(())
}

fn reinit_mount_namespace(mounts: &[Mount]) -> Result<()> {
    // Remount file system.
    for mount in mounts {
        let flags = MsFlags::MS_REMOUNT | MsFlags::MS_BIND | mount.ms_flags();
        syscall::mount(&mount.container_path, &mount.container_path, flags)?;
    }

    // Mount a new tmpfs.
    syscall::mkdir("/tmp")?;
    syscall::mount_tmpfs("/tmp")?;

    // Mount a new proc.
    syscall::mount_proc(Mount::PROC_DIR.1)?;
    syscall::unmount(Mount::PUT_OLD_PROC_DIR.1)?;
    syscall::rmdir(Mount::PUT_OLD_PROC_DIR.1)?;

    // Remount WORK_DIR as a read-write data volume.
    let flags = MsFlags::MS_REMOUNT | MsFlags::MS_BIND;
    syscall::mount(Mount::WORK_DIR.1, Mount::WORK_DIR.1, flags)?;

    // Switch to the working directory.
    syscall::chdir(Mount::WORK_DIR.1)
}

fn reinit_user_namespace(uid_mappings: &IDMap, gid_mappings: &IDMap) -> Result<()> {
    syscall::write("/proc/self/uid_map", &format!("{}\n", uid_mappings))?;
    syscall::write("/proc/self/setgroups", "deny")?;
    syscall::write("/proc/self/gid_map", &format!("{}\n", gid_mappings))
}
