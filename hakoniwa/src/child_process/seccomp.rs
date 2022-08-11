use libseccomp::{ScmpFilterContext, ScmpSyscall};

use crate::{child_process::error::Result, Seccomp};

pub fn init(seccomp: &Option<Seccomp>) -> Result<()> {
    if let Some(seccomp) = seccomp {
        let mut scmp_filter = ScmpFilterContext::new_filter(seccomp.dismatch_action())?;
        for syscall in seccomp.syscalls.iter() {
            let syscall = ScmpSyscall::from_name(syscall)?;
            scmp_filter.add_rule(seccomp.match_action(), syscall)?;
        }
        scmp_filter.load()?;
    }
    Ok(())
}
