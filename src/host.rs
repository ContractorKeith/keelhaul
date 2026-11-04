//! Machine and effective-user identity, independent of shell environment variables.

pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

pub fn username() -> String {
    let uid = uzers::get_effective_uid();
    uzers::get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| uid.to_string())
}
