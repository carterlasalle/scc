//! Engine multi-repo stitching (values; CLI renders).

// trace:exempt reason=internal-detail
pub fn stitch(
    members: &[std::path::PathBuf],
) -> crate::Result<(Vec<scc_store::system::Member>, Vec<scc_store::system::Stitch>)> {
    let roots: Vec<&std::path::Path> = members.iter().map(|p| p.as_path()).collect();
    let sys = scc_store::system::System::open(&roots)?;
    let mut all = sys.stitch_routes()?;
    all.extend(sys.stitch_topics()?);
    all.extend(sys.stitch_package_exports()?);
    Ok((sys.members, all))
}
