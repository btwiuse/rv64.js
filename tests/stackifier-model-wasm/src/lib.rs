use std::hint::black_box;

type Node = usize;
const CFG_MARKER: u32 = 0x3147_4643;
const MAX_REAL_NODES: usize = 512;

#[derive(Clone, Debug)]
struct GraphInput {
    successors: Vec<Vec<Node>>,
    entries: Vec<Node>,
    duplication_limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Shape {
    Basic(Node),
    Dispatcher(Vec<Node>),
    Loop(Vec<Shape>),
    Block(Vec<Shape>),
}

impl Shape {
    fn head(&self) -> Vec<Node> {
        match self {
            Self::Basic(node) => vec![*node],
            Self::Dispatcher(entries) => entries.clone(),
            Self::Loop(children) | Self::Block(children) => {
                children.first().map_or_else(Vec::new, Self::head)
            }
        }
    }
}

mod tree {
    use super::{Node, Shape};
    use std::collections::{BTreeMap, BTreeSet};

    const ENTRY_NODE: Node = usize::MAX;
    type Set = BTreeSet<Node>;
    type Graph = BTreeMap<Node, Set>;

    pub(super) fn stackify(
        successors: &[Vec<Node>],
        entries: &[Node],
        duplication_limit: usize,
    ) -> Vec<Shape> {
        let mut graph = Graph::new();
        for (node, outgoing) in successors.iter().enumerate() {
            graph.insert(
                node,
                outgoing
                    .iter()
                    .copied()
                    .filter(|target| *target < successors.len())
                    .collect(),
            );
        }
        graph.insert(
            ENTRY_NODE,
            entries
                .iter()
                .copied()
                .filter(|entry| *entry < successors.len())
                .collect(),
        );

        let mut result = loopify(&graph, duplication_limit);
        blockify(&mut result, &graph);
        result
    }

    fn branches(structure: &Shape, graph: &Graph, out: &mut Set) {
        match structure {
            Shape::Basic(node) => {
                if let Some(successors) = graph.get(node) {
                    out.extend(successors);
                }
            }
            Shape::Dispatcher(entries) => out.extend(entries),
            Shape::Loop(children) | Shape::Block(children) => {
                for child in children {
                    branches(child, graph, out);
                }
            }
        }
    }

    fn reverse(graph: &Graph) -> Graph {
        let mut reversed: Graph = graph.keys().map(|node| (*node, Set::new())).collect();
        for (&source, targets) in graph {
            for &target in targets {
                reversed.entry(target).or_default().insert(source);
            }
        }
        reversed
    }

    fn strongly_connected_components(graph: &Graph, reversed: &Graph) -> Vec<Vec<Node>> {
        fn visit(node: Node, graph: &Graph, seen: &mut Set, finished: &mut Vec<Node>) {
            if !seen.insert(node) {
                return;
            }
            if let Some(successors) = graph.get(&node) {
                for &successor in successors {
                    visit(successor, graph, seen, finished);
                }
            }
            finished.push(node);
        }

        fn assign(node: Node, reversed: &Graph, assigned: &mut Set, group: &mut Vec<Node>) {
            if !assigned.insert(node) {
                return;
            }
            group.push(node);
            if let Some(predecessors) = reversed.get(&node) {
                for &predecessor in predecessors {
                    assign(predecessor, reversed, assigned, group);
                }
            }
        }

        let mut finished = Vec::with_capacity(graph.len());
        let mut seen = Set::new();
        for &node in graph.keys() {
            visit(node, graph, &mut seen, &mut finished);
        }

        let mut assigned = Set::new();
        let mut groups = Vec::new();
        for &node in finished.iter().rev() {
            let mut group = Vec::new();
            assign(node, reversed, &mut assigned, &mut group);
            if !group.is_empty() {
                groups.push(group);
            }
        }
        groups
    }

    fn loopify(graph: &Graph, duplication_limit: usize) -> Vec<Shape> {
        let reversed = reverse(graph);
        let groups = strongly_connected_components(graph, &reversed);
        let mut result = Vec::new();

        for group in groups {
            if group.len() == 1 {
                let node = group[0];
                if node == ENTRY_NODE {
                    result.push(Shape::Dispatcher(
                        graph
                            .get(&ENTRY_NODE)
                            .into_iter()
                            .flatten()
                            .copied()
                            .collect(),
                    ));
                } else {
                    let basic = Shape::Basic(node);
                    if graph
                        .get(&node)
                        .is_some_and(|successors| successors.contains(&node))
                    {
                        result.push(Shape::Loop(vec![basic]));
                    } else {
                        result.push(basic);
                    }
                }
                continue;
            }

            let members: Set = group.iter().copied().collect();
            let entries: Vec<Node> = group
                .iter()
                .copied()
                .filter(|node| {
                    reversed.get(node).is_some_and(|predecessors| {
                        predecessors
                            .iter()
                            .any(|incoming| !members.contains(incoming))
                    })
                })
                .collect();

            if entries.len().saturating_mul(group.len()) > duplication_limit {
                let entry_set: Set = entries.iter().copied().collect();
                let mut subgroup = Graph::new();
                for &node in &group {
                    subgroup.insert(
                        node,
                        graph[&node]
                            .iter()
                            .copied()
                            .filter(|target| {
                                members.contains(target) && !entry_set.contains(target)
                            })
                            .collect(),
                    );
                }
                let mut children = loopify(&subgroup, duplication_limit);
                if entries.len() > 1 {
                    children.insert(0, Shape::Dispatcher(entries));
                }
                result.push(Shape::Loop(children));
            } else {
                for entry in entries {
                    let mut subgroup = Graph::new();
                    for &node in &group {
                        subgroup.insert(
                            node,
                            graph[&node]
                                .iter()
                                .copied()
                                .filter(|target| members.contains(target) && *target != entry)
                                .collect(),
                        );
                    }
                    result.push(Shape::Loop(loopify(&subgroup, duplication_limit)));
                }
            }
        }
        result
    }

    fn blockify(structures: &mut Vec<Shape>, graph: &Graph) {
        let mut index = 0;
        while index < structures.len() {
            match &mut structures[index] {
                Shape::Loop(children) | Shape::Block(children) => blockify(children, graph),
                Shape::Basic(_) | Shape::Dispatcher(_) => {}
            }

            let heads: Set = structures[index].head().into_iter().collect();
            let source = (0..index).find(|&candidate| {
                let mut outgoing = Set::new();
                branches(&structures[candidate], graph, &mut outgoing);
                !outgoing.is_disjoint(&heads)
            });
            let Some(source) = source else {
                index += 1;
                continue;
            };

            if source + 1 == index && matches!(structures[source], Shape::Basic(_)) {
                index += 1;
                continue;
            }

            let children: Vec<_> = structures.drain(source..index).collect();
            structures.insert(source, Shape::Block(children));
            index = source + 2;
        }
    }
}

mod dense {
    use super::{Node, Shape, MAX_REAL_NODES};

    const MAX_NODES_WITH_ENTRY: usize = MAX_REAL_NODES + 1;
    const WORDS: usize = MAX_NODES_WITH_ENTRY.div_ceil(64);

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct Set {
        words: [u64; WORDS],
    }

    impl Set {
        fn insert(&mut self, node: Node) -> bool {
            debug_assert!(node < MAX_NODES_WITH_ENTRY);
            let mask = 1_u64 << (node & 63);
            let word = &mut self.words[node >> 6];
            let inserted = *word & mask == 0;
            *word |= mask;
            inserted
        }

        fn contains(&self, node: Node) -> bool {
            node < MAX_NODES_WITH_ENTRY && self.words[node >> 6] & (1_u64 << (node & 63)) != 0
        }

        fn extend(&mut self, other: &Self) {
            for (left, right) in self.words.iter_mut().zip(other.words) {
                *left |= right;
            }
        }

        fn is_disjoint(&self, other: &Self) -> bool {
            self.words
                .iter()
                .zip(other.words)
                .all(|(left, right)| left & right == 0)
        }

        fn iter(&self) -> SetIter<'_> {
            SetIter {
                set: self,
                word: 0,
                remaining: 0,
            }
        }

        fn from_nodes(nodes: impl IntoIterator<Item = Node>) -> Self {
            let mut result = Self::default();
            for node in nodes {
                result.insert(node);
            }
            result
        }
    }

    struct SetIter<'a> {
        set: &'a Set,
        word: usize,
        remaining: u64,
    }

    impl Iterator for SetIter<'_> {
        type Item = Node;

        fn next(&mut self) -> Option<Self::Item> {
            loop {
                if self.remaining != 0 {
                    let bit = self.remaining.trailing_zeros() as usize;
                    self.remaining &= self.remaining - 1;
                    return Some(((self.word - 1) << 6) + bit);
                }
                if self.word == WORDS {
                    return None;
                }
                self.remaining = self.set.words[self.word];
                self.word += 1;
            }
        }
    }

    #[derive(Clone)]
    struct Graph {
        limit: usize,
        synthetic_entry: Option<Node>,
        present: Set,
        successors: Vec<Set>,
    }

    impl Graph {
        fn new(limit: usize, synthetic_entry: Option<Node>) -> Self {
            Self {
                limit,
                synthetic_entry,
                present: Set::default(),
                successors: vec![Set::default(); limit],
            }
        }

        fn insert(&mut self, node: Node, successors: Set) {
            debug_assert!(node < self.limit);
            self.present.insert(node);
            self.successors[node] = successors;
        }

        fn get(&self, node: Node) -> Option<&Set> {
            self.present.contains(node).then(|| &self.successors[node])
        }

        fn keys(&self) -> SetIter<'_> {
            self.present.iter()
        }

        fn len(&self) -> usize {
            self.present.iter().count()
        }
    }

    pub(super) fn stackify(
        successors: &[Vec<Node>],
        entries: &[Node],
        duplication_limit: usize,
    ) -> Vec<Shape> {
        assert!(successors.len() <= MAX_REAL_NODES);
        let entry = successors.len();
        let mut graph = Graph::new(entry + 1, Some(entry));
        for (node, outgoing) in successors.iter().enumerate() {
            graph.insert(
                node,
                Set::from_nodes(
                    outgoing
                        .iter()
                        .copied()
                        .filter(|target| *target < successors.len()),
                ),
            );
        }
        graph.insert(
            entry,
            Set::from_nodes(
                entries
                    .iter()
                    .copied()
                    .filter(|candidate| *candidate < successors.len()),
            ),
        );

        let mut result = loopify(&graph, duplication_limit);
        blockify(&mut result, &graph);
        result
    }

    fn branches(structure: &Shape, graph: &Graph, out: &mut Set) {
        match structure {
            Shape::Basic(node) => {
                if let Some(successors) = graph.get(*node) {
                    out.extend(successors);
                }
            }
            Shape::Dispatcher(entries) => {
                for &entry in entries {
                    out.insert(entry);
                }
            }
            Shape::Loop(children) | Shape::Block(children) => {
                for child in children {
                    branches(child, graph, out);
                }
            }
        }
    }

    fn reverse(graph: &Graph) -> Graph {
        let mut reversed = Graph::new(graph.limit, graph.synthetic_entry);
        for node in graph.keys() {
            reversed.insert(node, Set::default());
        }
        for source in graph.keys() {
            for target in graph.successors[source].iter() {
                reversed.successors[target].insert(source);
            }
        }
        reversed
    }

    fn strongly_connected_components(graph: &Graph, reversed: &Graph) -> Vec<Vec<Node>> {
        fn visit(node: Node, graph: &Graph, seen: &mut Set, finished: &mut Vec<Node>) {
            if !seen.insert(node) {
                return;
            }
            if let Some(successors) = graph.get(node) {
                for successor in successors.iter() {
                    visit(successor, graph, seen, finished);
                }
            }
            finished.push(node);
        }

        fn assign(node: Node, reversed: &Graph, assigned: &mut Set, group: &mut Vec<Node>) {
            if !assigned.insert(node) {
                return;
            }
            group.push(node);
            if let Some(predecessors) = reversed.get(node) {
                for predecessor in predecessors.iter() {
                    assign(predecessor, reversed, assigned, group);
                }
            }
        }

        let mut finished = Vec::with_capacity(graph.len());
        let mut seen = Set::default();
        for node in graph.keys() {
            visit(node, graph, &mut seen, &mut finished);
        }

        let mut assigned = Set::default();
        let mut groups = Vec::new();
        for &node in finished.iter().rev() {
            let mut group = Vec::new();
            assign(node, reversed, &mut assigned, &mut group);
            if !group.is_empty() {
                groups.push(group);
            }
        }
        groups
    }

    fn loopify(graph: &Graph, duplication_limit: usize) -> Vec<Shape> {
        let reversed = reverse(graph);
        let groups = strongly_connected_components(graph, &reversed);
        let mut result = Vec::new();

        for group in groups {
            if group.len() == 1 {
                let node = group[0];
                if graph.synthetic_entry == Some(node) {
                    result.push(Shape::Dispatcher(
                        graph.get(node).into_iter().flat_map(Set::iter).collect(),
                    ));
                } else {
                    let basic = Shape::Basic(node);
                    if graph
                        .get(node)
                        .is_some_and(|successors| successors.contains(node))
                    {
                        result.push(Shape::Loop(vec![basic]));
                    } else {
                        result.push(basic);
                    }
                }
                continue;
            }

            let members = Set::from_nodes(group.iter().copied());
            let entries: Vec<Node> = group
                .iter()
                .copied()
                .filter(|node| {
                    reversed.get(*node).is_some_and(|predecessors| {
                        predecessors
                            .iter()
                            .any(|incoming| !members.contains(incoming))
                    })
                })
                .collect();

            if entries.len().saturating_mul(group.len()) > duplication_limit {
                let entry_set = Set::from_nodes(entries.iter().copied());
                let mut subgroup = Graph::new(graph.limit, None);
                for &node in &group {
                    subgroup.insert(
                        node,
                        Set::from_nodes(graph.successors[node].iter().filter(|target| {
                            members.contains(*target) && !entry_set.contains(*target)
                        })),
                    );
                }
                let mut children = loopify(&subgroup, duplication_limit);
                if entries.len() > 1 {
                    children.insert(0, Shape::Dispatcher(entries));
                }
                result.push(Shape::Loop(children));
            } else {
                for entry in entries {
                    let mut subgroup = Graph::new(graph.limit, None);
                    for &node in &group {
                        subgroup.insert(
                            node,
                            Set::from_nodes(
                                graph.successors[node]
                                    .iter()
                                    .filter(|target| members.contains(*target) && *target != entry),
                            ),
                        );
                    }
                    result.push(Shape::Loop(loopify(&subgroup, duplication_limit)));
                }
            }
        }
        result
    }

    fn blockify(structures: &mut Vec<Shape>, graph: &Graph) {
        let mut index = 0;
        while index < structures.len() {
            match &mut structures[index] {
                Shape::Loop(children) | Shape::Block(children) => blockify(children, graph),
                Shape::Basic(_) | Shape::Dispatcher(_) => {}
            }

            let heads = Set::from_nodes(structures[index].head());
            let source = (0..index).find(|&candidate| {
                let mut outgoing = Set::default();
                branches(&structures[candidate], graph, &mut outgoing);
                !outgoing.is_disjoint(&heads)
            });
            let Some(source) = source else {
                index += 1;
                continue;
            };

            if source + 1 == index && matches!(structures[source], Shape::Basic(_)) {
                index += 1;
                continue;
            }

            let children: Vec<_> = structures.drain(source..index).collect();
            structures.insert(source, Shape::Block(children));
            index = source + 2;
        }
    }
}

fn parse_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let value = u32::from_le_bytes(bytes.get(*offset..*offset + 4)?.try_into().ok()?);
    *offset += 4;
    Some(value)
}

fn parse_corpus(bytes: &[u8]) -> Option<Vec<GraphInput>> {
    let mut offset = 0;
    let mut corpus = Vec::new();
    while offset < bytes.len() {
        if parse_u32(bytes, &mut offset)? != CFG_MARKER {
            return None;
        }
        let nodes = parse_u32(bytes, &mut offset)? as usize;
        let entry_count = parse_u32(bytes, &mut offset)? as usize;
        let duplication_limit = parse_u32(bytes, &mut offset)? as usize;
        if nodes > MAX_REAL_NODES {
            return None;
        }
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            entries.push(parse_u32(bytes, &mut offset)? as usize);
        }
        let mut successors = Vec::with_capacity(nodes);
        for _ in 0..nodes {
            let count = parse_u32(bytes, &mut offset)? as usize;
            let mut outgoing = Vec::with_capacity(count);
            for _ in 0..count {
                outgoing.push(parse_u32(bytes, &mut offset)? as usize);
            }
            successors.push(outgoing);
        }
        corpus.push(GraphInput {
            successors,
            entries,
            duplication_limit,
        });
    }
    (!corpus.is_empty()).then_some(corpus)
}

fn digest_byte(state: &mut u64, byte: u8) {
    *state ^= u64::from(byte);
    *state = state.wrapping_mul(0x0000_0100_0000_01b3);
}

fn digest_u32(state: &mut u64, value: u32) {
    for byte in value.to_le_bytes() {
        digest_byte(state, byte);
    }
}

fn digest_shapes(state: &mut u64, shapes: &[Shape]) {
    digest_u32(state, shapes.len() as u32);
    for shape in shapes {
        match shape {
            Shape::Basic(node) => {
                digest_byte(state, 0);
                digest_u32(state, *node as u32);
            }
            Shape::Dispatcher(entries) => {
                digest_byte(state, 1);
                digest_u32(state, entries.len() as u32);
                for &entry in entries {
                    digest_u32(state, entry as u32);
                }
            }
            Shape::Loop(children) => {
                digest_byte(state, 2);
                digest_shapes(state, children);
            }
            Shape::Block(children) => {
                digest_byte(state, 3);
                digest_shapes(state, children);
            }
        }
    }
}

fn render_shapes(output: &mut Vec<u8>, shapes: &[Shape]) {
    output.extend_from_slice(&(shapes.len() as u32).to_le_bytes());
    for shape in shapes {
        match shape {
            Shape::Basic(node) => {
                output.push(0);
                output.extend_from_slice(&(*node as u32).to_le_bytes());
            }
            Shape::Dispatcher(entries) => {
                output.push(1);
                output.extend_from_slice(&(entries.len() as u32).to_le_bytes());
                for &entry in entries {
                    output.extend_from_slice(&(entry as u32).to_le_bytes());
                }
            }
            Shape::Loop(children) => {
                output.push(2);
                render_shapes(output, children);
            }
            Shape::Block(children) => {
                output.push(3);
                render_shapes(output, children);
            }
        }
    }
}

static mut INPUT: Vec<u8> = Vec::new();
static mut CORPUS: Vec<GraphInput> = Vec::new();
static mut OUTPUT: Vec<u8> = Vec::new();
static mut RESULT_SINK: u64 = 0;

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_input_reserve(length: u32) -> u32 {
    unsafe {
        INPUT.clear();
        INPUT.resize(length as usize, 0);
        INPUT.as_mut_ptr() as usize as u32
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_prepare() -> u32 {
    unsafe {
        let Some(parsed) = parse_corpus(&INPUT) else {
            CORPUS.clear();
            return u32::MAX;
        };
        CORPUS = parsed;
        CORPUS.len().try_into().unwrap_or(u32::MAX)
    }
}

#[allow(static_mut_refs)]
fn run_model(dense: bool, repetitions: u32) -> u64 {
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    unsafe {
        for _ in 0..repetitions {
            for input in &CORPUS {
                let shapes = if dense {
                    dense::stackify(&input.successors, &input.entries, input.duplication_limit)
                } else {
                    tree::stackify(&input.successors, &input.entries, input.duplication_limit)
                };
                digest_shapes(&mut digest, black_box(&shapes));
                black_box(shapes);
            }
        }
        RESULT_SINK = digest;
    }
    digest
}

#[no_mangle]
pub extern "C" fn model_run_tree(repetitions: u32) -> u64 {
    run_model(false, repetitions)
}

#[no_mangle]
pub extern "C" fn model_run_dense(repetitions: u32) -> u64 {
    run_model(true, repetitions)
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_validate() -> u32 {
    unsafe {
        for (index, input) in CORPUS.iter().enumerate() {
            let control =
                tree::stackify(&input.successors, &input.entries, input.duplication_limit);
            let candidate =
                dense::stackify(&input.successors, &input.entries, input.duplication_limit);
            if control != candidate {
                return index.try_into().unwrap_or(u32::MAX - 1) + 1;
            }
        }
    }
    0
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_render(dense: u32) -> u32 {
    unsafe {
        let mut rendered = Vec::new();
        for input in &CORPUS {
            let shapes = if dense != 0 {
                dense::stackify(&input.successors, &input.entries, input.duplication_limit)
            } else {
                tree::stackify(&input.successors, &input.entries, input.duplication_limit)
            };
            render_shapes(&mut rendered, &shapes);
        }
        OUTPUT = rendered;
        OUTPUT.len().try_into().unwrap_or(u32::MAX)
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_output_ptr() -> u32 {
    unsafe { OUTPUT.as_ptr() as usize as u32 }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn model_account(which: u32) -> u64 {
    unsafe {
        match which {
            0 => CORPUS.len() as u64,
            1 => CORPUS
                .iter()
                .map(|input| input.successors.len() as u64)
                .sum(),
            2 => CORPUS
                .iter()
                .flat_map(|input| &input.successors)
                .map(|outgoing| outgoing.len() as u64)
                .sum(),
            3 => CORPUS.iter().map(|input| input.entries.len() as u64).sum(),
            _ => 0,
        }
    }
}
