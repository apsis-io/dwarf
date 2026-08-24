use std::path::{Component as PathComponent, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use oxc_resolver::{ResolveOptions, Resolver as OxcResolver};

#[derive(Clone)]
pub(crate) struct Resolver {
    inner: Arc<Inner>,
}

struct Inner {
    root: PathBuf,
    entry_path: String,
    resolver: OxcResolver,
}

impl Resolver {
    pub(crate) fn new(entry: &Path, module_root: Option<&Path>) -> Result<Self> {
        let entry = entry
            .canonicalize()
            .with_context(|| format!("failed to resolve JS entry path {}", entry.display()))?;

        if !entry.is_file() {
            return Err(anyhow!("JS entry path is not a file: {}", entry.display()));
        }

        let root = match module_root {
            Some(root) => root
                .canonicalize()
                .with_context(|| format!("failed to resolve module root {}", root.display()))?,
            None => default_module_root(&entry)?,
        };

        if !root.is_dir() {
            return Err(anyhow!(
                "module root is not a directory: {}",
                root.display()
            ));
        }

        let relative_entry = entry.strip_prefix(&root).with_context(|| {
            format!(
                "JS entry path {} is not under module root {}",
                entry.display(),
                root.display()
            )
        })?;

        let entry_path = guest_absolute_path(relative_entry)?;
        let resolver = OxcResolver::new(ResolveOptions {
            condition_names: vec!["import".into(), "default".into()],
            extensions: vec![".mjs".into(), ".js".into()],
            main_fields: vec!["module".into(), "main".into()],
            node_path: false,
            symlinks: false,
            ..ResolveOptions::default()
        });

        Ok(Self {
            inner: Arc::new(Inner {
                root,
                entry_path,
                resolver,
            }),
        })
    }

    pub(crate) fn resolve(&self, referrer: &str, specifier: &str) -> Result<String> {
        let referrer = self.guest_path_to_host(referrer)?;
        let resolved = self
            .inner
            .resolver
            .resolve_file(&referrer, specifier)
            .with_context(|| {
                format!(
                    "filesystem module not found: failed to resolve JavaScript import {specifier:?} from {}",
                    referrer.display()
                )
            })?
            .path()
            .to_path_buf();
        // Lexically normalized, NOT canonicalized. `ResolveOptions` already
        // asks oxc for symlink-preserving paths (`symlinks: false`), and
        // canonicalizing here threw that away: pnpm, bun and nub all link
        // `node_modules/<pkg>` into a global content-addressed store, so the
        // canonical path lands outside the module root and every dependency
        // was rejected with "is not under module root". Only a flat npm
        // `node_modules` worked.
        let resolved = lexical_normalize(&resolved);
        let relative = resolved.strip_prefix(&self.inner.root).with_context(|| {
            format!(
                "resolved JavaScript module {} is not under module root {}",
                resolved.display(),
                self.inner.root.display()
            )
        })?;

        guest_absolute_path(relative)
    }

    pub(crate) fn load(&self, path: &str) -> Result<String> {
        let path = self.guest_path_to_host(path)?;
        std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read JavaScript module {}", path.display()))
    }

    fn guest_path_to_host(&self, path: &str) -> Result<PathBuf> {
        let path = path
            .strip_prefix('/')
            .ok_or_else(|| anyhow!("JavaScript module path must be absolute: {path}"))?;
        let path = lexical_normalize(&self.inner.root.join(path));
        // The containment check is LEXICAL, which is what lets a symlinked
        // package store work: `..` still cannot climb out of the root, but a
        // symlink the dependency tree itself points at is followed when the
        // file is read. Escaping by symlink is not a threat model this tool
        // has — it reads files the caller already chose to depend on.
        if !path.starts_with(&self.inner.root) {
            anyhow::bail!(
                "JavaScript module path {} escapes module root {}",
                path.display(),
                self.inner.root.display()
            );
        }
        Ok(path)
    }

    pub(crate) fn entry_path(&self) -> &str {
        &self.inner.entry_path
    }
}

/// The implicit module root: the ENTRY'S OWN DIRECTORY, and deliberately
/// not the process's current directory.
///
/// It used to be the cwd whenever the entry lived underneath it, falling
/// back to the entry's parent otherwise. That made the build depend on
/// where it was invoked from rather than on its inputs: the root decides
/// each module's guest-visible path (`guest_absolute_path`), so the same
/// entry built from the repository root and from /tmp produced
/// `/examples/hello.js` and `/hello.js` respectively. Different strings,
/// different allocation sizes, a different heap for Wizer to snapshot —
/// two builds of one input differing by 32 bytes, which is exactly what
/// reproducibility is not.
///
/// The entry's directory is the same answer from anywhere. A build that
/// needs a WIDER root — an entry in `src/` importing `../shared/x.js` —
/// says so with `--module-root`, which is an input like any other and
/// therefore reproducible. That is a narrower default than before: what
/// used to work implicitly by being run from the right directory now needs
/// the flag, and says so with the "outside the module root" error.
/// Resolve `.` and `..` by PATH ARITHMETIC alone — no filesystem, and in
/// particular no symlink resolution.
///
/// `canonicalize` would answer where a file really lives; this answers what
/// the module graph called it. Those differ exactly when a package manager
/// links `node_modules/<pkg>` into a store, which is the common case for
/// pnpm/bun/nub, and the module root is a statement about the graph.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            PathComponent::CurDir => {}
            PathComponent::ParentDir => {
                // Pop a real segment; a leading `..` (nothing to pop) is kept
                // so the result stays outside any root and the caller's
                // containment check rejects it.
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn default_module_root(entry: &Path) -> Result<PathBuf> {
    entry
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("JS entry path has no parent: {}", entry.display()))
}

fn guest_absolute_path(relative: &Path) -> Result<String> {
    let mut guest = String::from("/");
    let mut first = true;

    for component in relative.components() {
        let PathComponent::Normal(part) = component else {
            return Err(anyhow!(
                "JS entry path contains unsupported component: {}",
                relative.display()
            ));
        };
        let part = part.to_str().ok_or_else(|| {
            anyhow!(
                "JS entry path contains non-UTF-8 component: {}",
                relative.display()
            )
        })?;

        if !first {
            guest.push('/');
        }
        guest.push_str(part);
        first = false;
    }

    if first {
        return Err(anyhow!("JS entry path cannot be the module root"));
    }

    Ok(guest)
}
