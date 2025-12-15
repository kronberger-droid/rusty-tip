use std::{collections::HashMap, ops::Deref};

/// Generate common aliases for signal names
fn generate_signal_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let name_lower = name.to_lowercase();

    // Common signal name patterns and their aliases
    let alias_patterns = [
        // Current signal variations
        ("current", vec!["i", "cur", "amp"]),
        ("bias", vec!["u", "voltage", "v"]),
        // Position signals
        ("x", vec!["x pos", "x position", "xpos"]),
        ("y", vec!["y pos", "y position", "ypos"]),
        ("z", vec!["z pos", "z position", "zpos", "height"]),
        // Frequency shift signals (common in AFM)
        (
            "oc m1 freq. shift",
            vec!["freq shift", "frequency shift", "df", "oc freq shift"],
        ),
        ("oc m1 amplitude", vec!["amplitude", "amp", "oc amp"]),
        ("oc m1 phase", vec!["phase", "oc phase"]),
        // Lock-in amplifier signals
        ("li demod 1 x", vec!["li1x", "demod1x", "x1"]),
        ("li demod 1 y", vec!["li1y", "demod1y", "y1"]),
        ("li demod 2 x", vec!["li2x", "demod2x", "x2"]),
        ("li demod 2 y", vec!["li2y", "demod2y", "y2"]),
        // Z controller
        ("z ctrl shift", vec!["z shift", "zshift"]),
        // Generic patterns
        ("frequency", vec!["freq", "f"]),
        ("amplitude", vec!["amp", "a"]),
        ("phase", vec!["ph"]),
        ("shift", vec!["sh"]),
    ];

    // Check for exact matches and add their aliases
    for (pattern, pattern_aliases) in &alias_patterns {
        if name_lower == *pattern {
            aliases.extend(pattern_aliases.iter().map(|s| s.to_string()));
        }
    }

    // Check for partial matches and create shortened versions
    for (pattern, pattern_aliases) in &alias_patterns {
        if name_lower.contains(pattern) {
            aliases.extend(pattern_aliases.iter().map(|s| s.to_string()));
        }
    }

    // Create abbreviated versions by removing common words
    let words_to_remove = ["the", "signal", "channel", "ch", "ctrl", "control"];
    let mut abbreviated = name_lower.clone();
    for word in &words_to_remove {
        abbreviated = abbreviated.replace(word, "").trim().to_string();
    }
    if abbreviated != name_lower && !abbreviated.is_empty() {
        aliases.push(abbreviated);
    }

    // Create initials-based aliases for multi-word signals
    let words: Vec<&str> = name_lower.split_whitespace().collect();
    if words.len() > 1 {
        let initials: String = words
            .iter()
            .map(|w| w.chars().next().unwrap_or('_'))
            .collect();
        if initials.len() > 1 {
            aliases.push(initials);
        }
    }

    // Remove duplicates and empty strings
    aliases.retain(|s| !s.is_empty());
    aliases.sort();
    aliases.dedup();

    aliases
}

/// Signal registry with case-insensitive lookup and TCP/Nanonis index mapping
#[derive(Debug, Clone, Default)]
pub struct SignalRegistry(HashMap<String, Signal>);

impl Deref for SignalRegistry {
    type Target = HashMap<String, Signal>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Signal {
    /// Original name read from nanonis host
    pub name: String,
    pub nanonis_index: u8,
    pub tcp_channel: Option<u8>,
}

impl Signal {
    pub fn get_name(&self) -> &String {
        &self.name
    }

    pub fn get_nanonis_index(&self) -> u8 {
        self.nanonis_index
    }

    pub fn get_tcp_channel(&self) -> Option<u8> {
        self.tcp_channel
    }
}

// #[derive(Debug, Clone)]
// pub struct SignalValue<'a> {
//     pub value: f32,
//     pub signal: &'a Signal,
// }

#[derive(Default)]
pub struct SignalRegistryBuilder {
    signals: HashMap<String, Signal>,
    nanonis_to_tcp: HashMap<u8, u8>,
}

impl SignalRegistryBuilder {
    pub fn add_tcp_mapping(mut self, nanonis_index: u8, tcp_channel: u8) -> Self {
        self.nanonis_to_tcp.insert(nanonis_index, tcp_channel);

        self
    }

    pub fn add_tcp_map(mut self, nanonis_to_tcp: &[(u8, u8)]) -> Self {
        nanonis_to_tcp.iter().for_each(|(n, t)| {
            self.nanonis_to_tcp.insert(*n, *t);
        });

        self
    }

    pub fn with_standard_map(mut self) -> Self {
        let standard_map: HashMap<u8, u8> = [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (24, 8),
            (25, 9),
            (26, 10),
            (27, 11),
            (28, 12),
            (29, 13),
            (30, 14),
            (31, 15),
            (74, 16),
            (75, 17),
            (76, 18),
            (77, 19),
            (78, 20),
            (79, 21),
            (80, 22),
            (81, 23),
        ]
        .iter()
        .cloned()
        .collect();

        self.nanonis_to_tcp.extend(standard_map);

        self
    }

    pub fn from_signal_names(mut self, signal_names: &[String]) -> Self {
        for (index, name) in signal_names.iter().enumerate() {
            let clean_name = name.split('(').next().unwrap().trim();

            let signal;

            if let Some(tcp_channel) = self.nanonis_to_tcp.get(&(index as u8)).copied() {
                signal = Signal {
                    name: name.clone(),
                    nanonis_index: index as u8,
                    tcp_channel: Some(tcp_channel),
                };
            } else {
                signal = Signal {
                    name: name.clone(),
                    nanonis_index: index as u8,
                    tcp_channel: None,
                };
            }
            self.signals.insert(name.to_lowercase(), signal.clone());

            if clean_name != name {
                self.signals
                    .insert(clean_name.to_lowercase(), signal.clone());
            }
        }
        self
    }

    pub fn add_signal(mut self, name: String, nanonis_index: u8) -> Self {
        let tcp_channel = self.nanonis_to_tcp.get(&nanonis_index).copied();
        let clean_name = name.split('(').next().unwrap().trim();

        let signal = Signal {
            name: name.clone(),
            nanonis_index,
            tcp_channel,
        };

        self.signals.insert(name.to_lowercase(), signal.clone());

        if clean_name != name {
            self.signals
                .insert(clean_name.to_lowercase(), signal.clone());
        };

        self
    }

    pub fn create_aliases(mut self) -> Self {
        let mut new_aliases = Vec::new();

        // Create aliases for existing signals
        for (existing_key, signal) in &self.signals {
            let name = &signal.name;
            let clean_name = name.split('(').next().unwrap_or(name).trim();

            // Create common aliases based on signal patterns
            let aliases = generate_signal_aliases(clean_name);

            for alias in aliases {
                let alias_key = alias.to_lowercase();
                // Only add if it doesn't already exist
                if !self.signals.contains_key(&alias_key) && alias_key != *existing_key {
                    new_aliases.push((alias_key, signal.clone()));
                }
            }
        }

        // Insert all new aliases
        for (alias_key, signal) in new_aliases {
            self.signals.insert(alias_key, signal);
        }

        self
    }

    pub fn build(self) -> SignalRegistry {
        SignalRegistry(self.signals)
    }
}

impl SignalRegistry {
    pub fn builder() -> SignalRegistryBuilder {
        SignalRegistryBuilder::default()
    }

    pub fn with_hardcoded_tcp_mapping(signal_names: &[String]) -> Self {
        Self::builder()
            .with_standard_map()
            .from_signal_names(signal_names)
            .create_aliases()
            .build()
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Signal> {
        self.0.get(&name.to_lowercase())
    }

    pub fn get_by_nanonis_index(&self, index: u8) -> Option<&Signal> {
        self.0.values().find(|s| s.nanonis_index == index)
    }

    pub fn all_names(&self) -> Vec<String> {
        self.0
            .values()
            .map(|s| s.name.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn tcp_signals(&self) -> Vec<&Signal> {
        self.0
            .values()
            .filter(|s| s.tcp_channel.is_some())
            .collect()
    }

    pub fn find_signals_like(&self, query: &str) -> Vec<&Signal> {
        let query_lower = query.to_lowercase();
        self.0
            .values()
            .filter(|s| s.name.to_lowercase().contains(&query_lower))
            .collect()
    }

    pub fn nanonis_to_tcp(
        &self,
        nanonis_index: crate::NanonisIndex,
    ) -> Result<crate::ChannelIndex, String> {
        self.0
            .values()
            .find(|s| s.nanonis_index == nanonis_index.get())
            .and_then(|s| s.tcp_channel)
            .map(|ch| crate::ChannelIndex::new_unchecked(ch))
            .ok_or_else(|| format!("No TCP channel for Nanonis index {}", nanonis_index.get()))
    }

    pub fn tcp_to_nanonis(
        &self,
        tcp_channel: crate::ChannelIndex,
    ) -> Result<crate::NanonisIndex, String> {
        self.0
            .values()
            .find(|s| s.tcp_channel == Some(tcp_channel.get()))
            .map(|s| crate::NanonisIndex::new_unchecked(s.nanonis_index))
            .ok_or_else(|| format!("No Nanonis index for TCP channel {}", tcp_channel.get()))
    }

    pub fn has_tcp_channel(&self, nanonis_index: crate::NanonisIndex) -> bool {
        self.0
            .values()
            .any(|s| s.nanonis_index == nanonis_index.get() && s.tcp_channel.is_some())
    }
}
