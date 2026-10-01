//! The scanned tree, stored as an arena of nodes addressed by index.

pub type NodeId = u32;
pub const NO_PARENT: NodeId = u32::MAX;

pub const FLAG_DIR: u8 = 1;
pub const FLAG_SYMLINK: u8 = 2;
/// The directory could not be opened (permissions and the like).
pub const FLAG_INACCESSIBLE: u8 = 4;
/// Taken out of the tree by the collector; still in the arena.
pub const FLAG_DETACHED: u8 = 8;

pub struct Node {
    pub name: Box<str>,
    pub parent: NodeId,
    /// Bytes on disk, including everything below a directory.
    pub size: u64,
    /// Number of non-directory items at or below this node.
    pub files: u64,
    pub children: Vec<NodeId>,
    pub flags: u8,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.flags & FLAG_DIR != 0
    }
}

/// One entry read from a directory, before it joins the tree.
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
}

pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    /// A tree holding just the root directory. The root's name is its full path.
    pub fn new(root_path: &str) -> Tree {
        Tree {
            nodes: vec![Node {
                name: root_path.into(),
                parent: NO_PARENT,
                size: 0,
                files: 0,
                children: Vec::new(),
                flags: FLAG_DIR,
            }],
        }
    }

    pub const ROOT: NodeId = 0;

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize)
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// Add the entries of one directory and roll their totals up to the root.
    /// Returns the ids of the new subdirectories, in entry order.
    pub fn attach(&mut self, parent: NodeId, entries: Vec<Entry>) -> Vec<NodeId> {
        let mut bytes = 0u64;
        let mut files = 0u64;
        let mut ids = Vec::with_capacity(entries.len());
        let mut dirs = Vec::new();
        for e in entries {
            let id = self.nodes.len() as NodeId;
            let mut flags = 0;
            if e.is_dir {
                flags |= FLAG_DIR;
                dirs.push(id);
            } else {
                bytes += e.size;
                files += 1;
            }
            if e.is_symlink {
                flags |= FLAG_SYMLINK;
            }
            self.nodes.push(Node {
                name: e.name.into_boxed_str(),
                parent,
                size: if e.is_dir { 0 } else { e.size },
                files: if e.is_dir { 0 } else { 1 },
                children: Vec::new(),
                flags,
            });
            ids.push(id);
        }
        self.nodes[parent as usize].children = ids;
        self.add_up(parent, bytes as i64, files as i64);
        dirs
    }

    fn add_up(&mut self, from: NodeId, bytes: i64, files: i64) {
        let mut cur = from;
        while cur != NO_PARENT {
            let n = &mut self.nodes[cur as usize];
            n.size = (n.size as i64 + bytes) as u64;
            n.files = (n.files as i64 + files) as u64;
            cur = n.parent;
        }
    }

    /// Full path of a node.
    pub fn path(&self, id: NodeId) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT {
            let n = self.node(cur);
            parts.push(&n.name);
            cur = n.parent;
        }
        let root = parts.pop().unwrap_or("");
        if parts.is_empty() {
            return root.to_string();
        }
        let sep = std::path::MAIN_SEPARATOR;
        let mut out = String::from(root);
        for p in parts.iter().rev() {
            if !out.ends_with(sep) && !out.ends_with('/') {
                out.push(sep);
            }
            out.push_str(p);
        }
        out
    }

    /// Name to show for a node: the last path component, also for the root.
    pub fn display_name(&self, id: NodeId) -> String {
        let n = self.node(id);
        if n.parent != NO_PARENT {
            return n.name.to_string();
        }
        let trimmed = n.name.trim_end_matches(['/', '\\']);
        match trimmed.rsplit(['/', '\\']).next() {
            Some(last) if !last.is_empty() => last.to_string(),
            _ => n.name.to_string(),
        }
    }

    /// Ancestors from the root down to `id`, inclusive.
    pub fn chain(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT {
            out.push(cur);
            cur = self.node(cur).parent;
        }
        out.reverse();
        out
    }

    pub fn is_descendant(&self, id: NodeId, of: NodeId) -> bool {
        let mut cur = id;
        while cur != NO_PARENT {
            if cur == of {
                return true;
            }
            cur = self.node(cur).parent;
        }
        false
    }

    /// Take a node out of its parent (the collector does this). Returns false
    /// for the root, unknown ids and nodes that are already out.
    pub fn detach(&mut self, id: NodeId) -> bool {
        let Some(n) = self.get(id) else { return false };
        let (parent, size, files) = (n.parent, n.size, n.files);
        if parent == NO_PARENT || n.flags & FLAG_DETACHED != 0 {
            return false;
        }
        let kids = &mut self.nodes[parent as usize].children;
        let Some(pos) = kids.iter().position(|&c| c == id) else { return false };
        kids.remove(pos);
        self.nodes[id as usize].flags |= FLAG_DETACHED;
        self.add_up(parent, -(size as i64), -(files as i64));
        true
    }

    /// Put a detached node back under its parent, keeping children sorted by size.
    pub fn reattach(&mut self, id: NodeId) -> bool {
        let Some(n) = self.get(id) else { return false };
        if n.flags & FLAG_DETACHED == 0 {
            return false;
        }
        let (parent, size, files) = (n.parent, n.size, n.files);
        let pos = {
            let kids = &self.nodes[parent as usize].children;
            kids.iter().position(|&c| self.node(c).size < size).unwrap_or(kids.len())
        };
        self.nodes[parent as usize].children.insert(pos, id);
        self.nodes[id as usize].flags &= !FLAG_DETACHED;
        self.add_up(parent, size as i64, files as i64);
        true
    }

    /// Sort every directory's children largest first.
    pub fn sort_all(&mut self) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].children.len() > 1 {
                let mut kids = std::mem::take(&mut self.nodes[i].children);
                kids.sort_by(|a, b| self.node(*b).size.cmp(&self.node(*a).size));
                self.nodes[i].children = kids;
            }
        }
    }

    /// Children of a node, largest first (sorted on the fly while a scan runs).
    pub fn sorted_children(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids = self.node(id).children.clone();
        kids.sort_by(|a, b| self.node(*b).size.cmp(&self.node(*a).size));
        kids
    }

    /// Visit every file at or below `id`.
    pub fn walk_files(&self, id: NodeId, mut visit: impl FnMut(NodeId, &Node)) {
        let n = self.node(id);
        if !n.is_dir() {
            visit(id, n);
            return;
        }
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            for &c in &self.node(cur).children {
                let cn = self.node(c);
                if cn.is_dir() {
                    stack.push(c);
                } else {
                    visit(c, cn);
                }
            }
        }
    }
}
