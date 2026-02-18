use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// -------------------- Raft roles --------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Follower,
    Candidate,
    Leader,
}

// -------------------- Commands + log --------------------

#[derive(Debug, Clone)]
enum Command {
    Put { key: String, value: String },
}

#[derive(Debug, Clone)]
struct LogEntry {
    term: u32,
    cmd: Command,
}

// -------------------- Messages (Raft RPCs) --------------------

#[derive(Debug, Clone)]
enum Msg {
    RequestVote {
        term: u32,
        candidate_id: u32,
        last_log_index: usize,
        last_log_term: u32,
    },
    RequestVoteReply {
        term: u32,
        from: u32,
        vote_granted: bool,
    },

    AppendEntries {
        term: u32,
        leader_id: u32,
        prev_log_index: usize,
        prev_log_term: u32,
        entries: Vec<LogEntry>,
        leader_commit: usize,
    },
    AppendEntriesReply {
        term: u32,
        from: u32,
        success: bool,
        match_index: usize,
    },
}

// -------------------- Network chaos layer --------------------

#[derive(Debug, Clone)]
struct ChaosConfig {
    drop_rate: f64,          // 0.0..1.0
    min_delay_ms: u64,       // e.g., 0
    max_delay_ms: u64,       // e.g., 200
}

#[derive(Debug)]
struct Network {
    inbox: HashMap<u32, mpsc::Sender<(u32, Msg)>>, // to -> sender of (from, msg)
    chaos: ChaosConfig,

    // Partition map: node_id -> partition_id (0,1,...)
    // If two nodes have different partition_id, messages between them are dropped.
    partition: Mutex<HashMap<u32, u32>>,
}

impl Network {
    fn new(chaos: ChaosConfig) -> Self {
        Self {
            inbox: HashMap::new(),
            chaos,
            partition: Mutex::new(HashMap::new()),
        }
    }

    fn register(&mut self, node_id: u32, tx: mpsc::Sender<(u32, Msg)>) {
        self.inbox.insert(node_id, tx);
        self.partition.lock().unwrap().insert(node_id, 0); // default: all connected
    }

    fn set_partition_groups(&self, group0: &[u32], group1: &[u32]) {
        let mut p = self.partition.lock().unwrap();
        for &id in group0 {
            p.insert(id, 0);
        }
        for &id in group1 {
            p.insert(id, 1);
        }
        println!("🧱 PARTITION ON: group0={:?} group1={:?}", group0, group1);
    }

    fn heal(&self) {
        let mut p = self.partition.lock().unwrap();
        for (_, v) in p.iter_mut() {
            *v = 0;
        }
        println!("🩹 PARTITION HEALED: all nodes connected");
    }

    fn can_deliver(&self, from: u32, to: u32) -> bool {
        let p = self.partition.lock().unwrap();
        let pf = p.get(&from).copied().unwrap_or(0);
        let pt = p.get(&to).copied().unwrap_or(0);
        pf == pt
    }

    fn send(&self, from: u32, to: u32, msg: Msg) {
        // Partition drop
        if !self.can_deliver(from, to) {
            return;
        }

        // Random drop
        if rand::thread_rng().gen_bool(self.chaos.drop_rate) {
            return;
        }

        // Random delay
        let delay = if self.chaos.max_delay_ms <= self.chaos.min_delay_ms {
            self.chaos.min_delay_ms
        } else {
            rand::thread_rng().gen_range(self.chaos.min_delay_ms..=self.chaos.max_delay_ms)
        };

        let tx = match self.inbox.get(&to) {
            Some(tx) => tx.clone(),
            None => return,
        };

        thread::spawn(move || {
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
            let _ = tx.send((from, msg));
        });
    }

    fn broadcast(&self, from: u32, peers: &[u32], msg: Msg) {
        for &to in peers {
            self.send(from, to, msg.clone());
        }
    }
}

// -------------------- Node state --------------------

#[derive(Debug)]
struct Node {
    id: u32,
    peers: Vec<u32>,

    role: Role,
    current_term: u32,
    voted_for: Option<u32>,
    leader_id: Option<u32>,

    log: Vec<LogEntry>, // idx 0 dummy
    commit_index: usize,
    last_applied: usize,

    kv: HashMap<String, String>,

    votes: HashSet<u32>,

    next_index: HashMap<u32, usize>,
    match_index: HashMap<u32, usize>,

    election_deadline: Instant,
    heartbeat_deadline: Instant,
    client_deadline: Instant,
}

impl Node {
    fn new(id: u32, peers: Vec<u32>) -> Self {
        let now = Instant::now();
        let dummy = LogEntry {
            term: 0,
            cmd: Command::Put {
                key: "__dummy__".into(),
                value: "__dummy__".into(),
            },
        };

        let mut n = Self {
            id,
            peers,
            role: Role::Follower,
            current_term: 0,
            voted_for: None,
            leader_id: None,

            log: vec![dummy],
            commit_index: 0,
            last_applied: 0,

            kv: HashMap::new(),

            votes: HashSet::new(),
            next_index: HashMap::new(),
            match_index: HashMap::new(),

            election_deadline: now,
            heartbeat_deadline: now + Duration::from_secs(3600),
            client_deadline: now + Duration::from_secs(3600),
        };
        n.reset_election_deadline();
        n
    }

    fn majority(&self) -> usize {
        (self.peers.len() + 1) / 2 + 1
    }

    fn reset_election_deadline(&mut self) {
        let ms = rand::thread_rng().gen_range(1200..2500);
        self.election_deadline = Instant::now() + Duration::from_millis(ms);
    }

    fn reset_heartbeat_deadline(&mut self) {
        self.heartbeat_deadline = Instant::now() + Duration::from_millis(250);
    }

    fn reset_client_deadline(&mut self) {
        let ms = rand::thread_rng().gen_range(900..1400);
        self.client_deadline = Instant::now() + Duration::from_millis(ms);
    }

    fn last_log_index(&self) -> usize {
        self.log.len() - 1
    }

    fn last_log_term(&self) -> u32 {
        self.log[self.last_log_index()].term
    }

    fn term_at(&self, idx: usize) -> u32 {
        self.log.get(idx).map(|e| e.term).unwrap_or(0)
    }

    fn become_follower(&mut self, new_term: u32, leader: Option<u32>) {
        if self.role != Role::Follower {
            println!("⬇️  Node {} steps down to Follower (term {})", self.id, new_term);
        }
        self.role = Role::Follower;
        self.current_term = new_term;
        self.voted_for = None;
        self.leader_id = leader;
        self.votes.clear();
        self.next_index.clear();
        self.match_index.clear();
        self.reset_election_deadline();
        self.heartbeat_deadline = Instant::now() + Duration::from_secs(3600);
        self.client_deadline = Instant::now() + Duration::from_secs(3600);
    }

    fn start_election(&mut self, net: &Network) {
        self.role = Role::Candidate;
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.leader_id = None;

        self.votes.clear();
        self.votes.insert(self.id);
        self.reset_election_deadline();

        let lli = self.last_log_index();
        let llt = self.last_log_term();

        println!(
            "🗳️  Node {} starts election term {} (last idx={}, last term={})",
            self.id, self.current_term, lli, llt
        );

        for &p in &self.peers {
            net.send(
                self.id,
                p,
                Msg::RequestVote {
                    term: self.current_term,
                    candidate_id: self.id,
                    last_log_index: lli,
                    last_log_term: llt,
                },
            );
        }
    }

    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.leader_id = Some(self.id);

        let next = self.last_log_index() + 1;
        self.next_index.clear();
        self.match_index.clear();
        for &p in &self.peers {
            self.next_index.insert(p, next);
            self.match_index.insert(p, 0);
        }

        self.reset_heartbeat_deadline();
        self.reset_client_deadline();

        println!("👑 Node {} becomes LEADER term {}", self.id, self.current_term);
    }

    fn leader_tick(&mut self, net: &Network) {
        if self.role != Role::Leader {
            return;
        }
        let now = Instant::now();

        if now >= self.heartbeat_deadline {
            self.replicate_to_all(net);
            self.reset_heartbeat_deadline();
        }

        if now >= self.client_deadline {
            self.append_client_command();
            self.replicate_to_all(net);
            self.reset_client_deadline();
        }
    }

    fn append_client_command(&mut self) {
        let k = format!("k{}", rand::thread_rng().gen_range(1..5));
        let v = format!("v{}_t{}", rand::thread_rng().gen_range(10..99), self.current_term);

        self.log.push(LogEntry {
            term: self.current_term,
            cmd: Command::Put { key: k, value: v },
        });

        println!(
            "✍️  Leader {} appended idx {} (term {})",
            self.id,
            self.last_log_index(),
            self.current_term
        );
    }

    fn replicate_to_all(&mut self, net: &Network) {
        let term = self.current_term;
        let leader_id = self.id;

        for &p in &self.peers {
            let ni = self.next_index.get(&p).copied().unwrap_or(1);
            let prev = ni - 1;
            let prev_term = self.term_at(prev);

            let entries: Vec<LogEntry> = if ni <= self.last_log_index() {
                self.log[ni..=self.last_log_index()].to_vec()
            } else {
                vec![]
            };

            net.send(
                self.id,
                p,
                Msg::AppendEntries {
                    term,
                    leader_id,
                    prev_log_index: prev,
                    prev_log_term: prev_term,
                    entries,
                    leader_commit: self.commit_index,
                },
            );
        }
    }

    fn advance_commit_index(&mut self) {
        let last = self.last_log_index();
        let mut candidate = self.commit_index;

        for n in (self.commit_index + 1)..=last {
            if self.log[n].term != self.current_term {
                continue;
            }
            let mut count = 1; // leader itself
            for (_, &mi) in self.match_index.iter() {
                if mi >= n {
                    count += 1;
                }
            }
            if count >= self.majority() {
                candidate = n;
            }
        }

        if candidate > self.commit_index {
            self.commit_index = candidate;
            println!(
                "✅ Leader {} committed up to idx {} (term {})",
                self.id, self.commit_index, self.current_term
            );
            self.apply_committed();
        }
    }

    fn apply_committed(&mut self) {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            let entry = self.log[self.last_applied].clone();
            match entry.cmd {
                Command::Put { key, value } => {
                    self.kv.insert(key, value);
                }
            }
        }
    }

    fn handle(&mut self, from: u32, msg: Msg, net: &Network) {
        match msg {
            Msg::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                if term < self.current_term {
                    net.send(
                        self.id,
                        candidate_id,
                        Msg::RequestVoteReply {
                            term: self.current_term,
                            from: self.id,
                            vote_granted: false,
                        },
                    );
                    return;
                }

                if term > self.current_term {
                    self.become_follower(term, None);
                }

                let my_term = self.last_log_term();
                let my_idx = self.last_log_index();
                let up_to_date = (last_log_term > my_term)
                    || (last_log_term == my_term && last_log_index >= my_idx);

                let can_vote =
                    (self.voted_for.is_none() || self.voted_for == Some(candidate_id)) && up_to_date;

                if can_vote {
                    self.voted_for = Some(candidate_id);
                    self.reset_election_deadline();
                }

                net.send(
                    self.id,
                    candidate_id,
                    Msg::RequestVoteReply {
                        term: self.current_term,
                        from: self.id,
                        vote_granted: can_vote,
                    },
                );
            }

            Msg::RequestVoteReply {
                term,
                from,
                vote_granted,
            } => {
                if term > self.current_term {
                    self.become_follower(term, None);
                    return;
                }
                if self.role == Role::Candidate && term == self.current_term && vote_granted {
                    self.votes.insert(from);
                    if self.votes.len() >= self.majority() {
                        self.become_leader();
                    }
                }
            }

            Msg::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                if term < self.current_term {
                    net.send(
                        self.id,
                        leader_id,
                        Msg::AppendEntriesReply {
                            term: self.current_term,
                            from: self.id,
                            success: false,
                            match_index: 0,
                        },
                    );
                    return;
                }

                // Accept leader for this term
                if term > self.current_term {
                    self.become_follower(term, Some(leader_id));
                } else if self.role != Role::Follower {
                    self.become_follower(term, Some(leader_id));
                } else {
                    self.leader_id = Some(leader_id);
                }

                self.reset_election_deadline();

                // Consistency check
                if prev_log_index > self.last_log_index()
                    || self.term_at(prev_log_index) != prev_log_term
                {
                    net.send(
                        self.id,
                        leader_id,
                        Msg::AppendEntriesReply {
                            term: self.current_term,
                            from: self.id,
                            success: false,
                            match_index: 0,
                        },
                    );
                    return;
                }

                // Conflict truncate
                let mut idx = prev_log_index + 1;
                for e in &entries {
                    if idx <= self.last_log_index() && self.term_at(idx) != e.term {
                        self.log.truncate(idx);
                        break;
                    }
                    idx += 1;
                }

                // Append new entries
                let start = prev_log_index + 1;
                for (i, e) in entries.into_iter().enumerate() {
                    let pos = start + i;
                    if pos > self.last_log_index() {
                        self.log.push(e);
                    }
                }

                // Commit + apply
                self.commit_index = self.commit_index.max(leader_commit.min(self.last_log_index()));
                self.apply_committed();

                let mi = self.last_log_index();
                net.send(
                    self.id,
                    leader_id,
                    Msg::AppendEntriesReply {
                        term: self.current_term,
                        from: self.id,
                        success: true,
                        match_index: mi,
                    },
                );
            }

            Msg::AppendEntriesReply {
                term,
                from,
                success,
                match_index,
            } => {
                if term > self.current_term {
                    self.become_follower(term, None);
                    return;
                }
                if self.role != Role::Leader || term != self.current_term {
                    return;
                }

                if success {
                    self.match_index.insert(from, match_index);
                    self.next_index.insert(from, match_index + 1);
                    self.advance_commit_index();
                } else {
                    // backoff and retry later
                    let ni = self.next_index.get(&from).copied().unwrap_or(1);
                    self.next_index.insert(from, ni.saturating_sub(1).max(1));
                }
            }
        }

        // unused but keeps "from" meaningful in demo
        let _ = from;
    }
}

// -------------------- Node event loop --------------------

fn run_node(id: u32, peers: Vec<u32>, net: Arc<Network>, rx: mpsc::Receiver<(u32, Msg)>) {
    let mut node = Node::new(id, peers);

    loop {
        let now = Instant::now();

        // next wake-up based on timers
        let mut next_deadline = node.election_deadline;
        if node.role == Role::Leader {
            next_deadline = next_deadline.min(node.heartbeat_deadline);
            next_deadline = next_deadline.min(node.client_deadline);
        }

        let wait = next_deadline
            .checked_duration_since(now)
            .unwrap_or(Duration::from_millis(0));

        match rx.recv_timeout(wait) {
            Ok((from, msg)) => node.handle(from, msg, &net),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now2 = Instant::now();

                if node.role != Role::Leader && now2 >= node.election_deadline {
                    node.start_election(&net);
                }

                node.leader_tick(&net);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// -------------------- Main: build cluster + chaos schedule --------------------

fn main() {
    let ids: Vec<u32> = vec![1, 2, 3, 4, 5];

    let chaos = ChaosConfig {
        drop_rate: 0.25, //0.10,     // 10% drop
        min_delay_ms: 50, //0,
        max_delay_ms: 400 //180,    // up to 180ms delay
    };

    let mut net0 = Network::new(chaos);

    // Each node gets an inbox receiver. Network holds the senders.
    let mut rxs: HashMap<u32, mpsc::Receiver<(u32, Msg)>> = HashMap::new();

    for &id in &ids {
        let (tx, rx) = mpsc::channel::<(u32, Msg)>();
        net0.register(id, tx);
        rxs.insert(id, rx);
    }

    let net = Arc::new(net0);

    // Spawn nodes
    for &id in &ids {
        let peers = ids.iter().copied().filter(|&x| x != id).collect::<Vec<_>>();
        let rx = rxs.remove(&id).unwrap();
        let net_clone = Arc::clone(&net);
        thread::spawn(move || run_node(id, peers, net_clone, rx));
    }

    // Chaos schedule (partition + heal)
    // After 3s: partition into {1,2} and {3,4,5} (majority on group1)
    // After 8s: heal.
    let net_ctrl = Arc::clone(&net);
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(3));
        net_ctrl.set_partition_groups(&[1, 2], &[3, 4, 5]);

        thread::sleep(Duration::from_secs(5));
        net_ctrl.heal();
    });

    // Run demo
    thread::sleep(Duration::from_secs(14));
    println!("✅ Day 4 demo finished (14s). Exiting.");
    std::process::exit(0);
}
