use std::{collections::HashMap, ops::Deref};

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
    name: String,
    nanonis_index: u16,
    tcp_channel: Option<u16>,
}

impl Signal {
    pub fn get_name() -> String {
        todo!()
    }

    pub fn get_nanonis_index() -> u16 {
        todo!()
    }

    pub fn get_tcp_channel() -> Option<u16> {
        todo!()
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
    nanonis_to_tcp: HashMap<u16, u16>,
}

impl SignalRegistryBuilder {
    pub fn add_tcp_mapping(mut self, nanonis_index: u16, tcp_channel: u16) -> Self {
        self.nanonis_to_tcp.insert(nanonis_index, tcp_channel);

        self
    }

    pub fn add_tcp_map(mut self, nanonis_to_tcp: &[(u16, u16)]) -> Self {
        nanonis_to_tcp
            .iter()
            .map(|(n, t)| self.nanonis_to_tcp.insert(*n, *t));

        self
    }

    pub fn with_standard_map(mut self) -> Self {
        let standard_map: HashMap<u16, u16> = [
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
            (72, 16),
            (74, 17),
            (75, 18),
            (76, 19),
            (77, 20),
            (78, 21),
            (79, 22),
            (80, 23),
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

            if let Some(tcp_channel) = self.nanonis_to_tcp.get(&(index as u16)).copied() {
                signal = Signal {
                    name: name.clone(),
                    nanonis_index: index as u16,
                    tcp_channel: Some(tcp_channel),
                };
            } else {
                signal = Signal {
                    name: name.clone(),
                    nanonis_index: index as u16,
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

    pub fn add_signal(mut self, name: String, nanonis_index: u16) -> Self {
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

    pub fn create_alisases(mut self) -> Self {
        todo!()
    }
}

impl SignalRegistry {
    pub fn builder() -> SignalRegistryBuilder {
        SignalRegistryBuilder::default()
    }
}
