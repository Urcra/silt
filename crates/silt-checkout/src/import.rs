use std::collections::BTreeMap;

use silt_crdt::{InodeId, InodeKind, Parent};
use silt_log::{Author, Content, Delta, Family, GitOp, LogError, Op};
use silt_types::{ActorId, GitOid, ThreadId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFile {
    pub path: String,
    pub oid: GitOid,
    pub executable: bool,
}

/// Starts a family's files as `commit`'s tree: a genesis marker, then one
/// create per directory and file. Files stay lazy git blobs until edited.
pub fn import(
    family: &mut Family,
    actor: ActorId,
    thread: ThreadId,
    commit: Option<(GitOid, Option<String>)>,
    files: &[ImportFile],
    time_ms: i64,
) -> Result<Vec<Delta>, LogError> {
    let mut deltas = Vec::new();
    let mut push = |family: &mut Family, op: Op| -> Result<InodeId, LogError> {
        let delta = family.delta(actor, thread, Author::Import, time_ms, family.thread_head(thread), op);
        let id = delta.span.first();
        family.apply(delta.clone(), &mut std::collections::HashMap::new())?;
        deltas.push(delta);
        Ok(InodeId(id))
    };
    if let Some((commit, branch)) = commit {
        push(family, Op::Git(GitOp::Genesis { commit, branch }))?;
    }
    let mut dirs: BTreeMap<String, InodeId> = BTreeMap::new();
    for file in files {
        let mut parent = Parent::Root;
        let mut components: Vec<&str> = file.path.split('/').collect();
        let name = components.pop().expect("split yields at least one component");
        let mut prefix = String::new();
        for component in components {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let dir = match dirs.get(&prefix) {
                Some(dir) => *dir,
                None => {
                    let dir = push(
                        family,
                        Op::Create {
                            kind: InodeKind::Dir,
                            parent,
                            name: component.to_owned(),
                            content: Content::Empty,
                            executable: false,
                        },
                    )?;
                    dirs.insert(prefix.clone(), dir);
                    dir
                }
            };
            parent = Parent::Dir(dir);
        }
        push(
            family,
            Op::Create {
                kind: InodeKind::File,
                parent,
                name: name.to_owned(),
                content: Content::GitText(file.oid),
                executable: file.executable,
            },
        )?;
    }
    Ok(deltas)
}
