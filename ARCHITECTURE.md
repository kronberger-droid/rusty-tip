# Nanonis Rust Library - System Architecture

## High-Level System Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            User Application                             │
└─────────────────────────────┬───────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          Controller                                     │
│  ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────────┐   │
│  │ Data Reading    │   │ State Enrichment│   │ Action Execution    │   │
│  │ Orchestration   │   │ (Position, etc) │   │ (Approach, Move)    │   │
│  └─────────────────┘   └─────────────────┘   └─────────────────────┘   │
└──────────────┬──────────────────────────────────────────┬───────────────┘
               │                                          │
               ▼                                          ▼
┌─────────────────────────┐                    ┌─────────────────────────┐
│     StateClassifier     │                    │     PolicyEngine        │
│                         │                    │                         │
│ ┌─────────────────────┐ │                    │ ┌─────────────────────┐ │
│ │ BoundaryClassifier  │ │                    │ │  RuleBasedPolicy    │ │
│ │                     │ │                    │ │                     │ │
│ │ • Signal buffering  │ │                    │ │ • Classification    │ │
│ │ • Drop-front logic  │ │                    │ │   → Decision        │ │
│ │ • Boundary checking │ │                    │ │ • Good/Bad/Stable   │ │
│ │ • Stability track   │ │                    │ │                     │ │
│ └─────────────────────┘ │                    │ └─────────────────────┘ │
└─────────────┬───────────┘                    └─────────────┬───────────┘
              │                                              │
              ▼                                              ▼
┌─────────────────────────┐                    ┌─────────────────────────┐
│       TipState          │                    │    PolicyDecision       │
│                         │                    │                         │
│ • Primary signal        │◄───────────────────┤ • Good                  │
│ • Signal history        │                    │ • Bad                   │
│ • Position context      │                    │ • Stable                │
│ • Classification        │                    │                         │
│ • Timestamp            │                    │                         │
└─────────────────────────┘                    └─────────────────────────┘
               ▲
               │
┌──────────────┴───────────────────────────────────────────────────────────┐
│                        NanonisClient                                     │
│                                                                          │
│  ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────────┐   │
│  │ Signal Reading  │   │ Position Control│   │ Motor Control       │   │
│  │ • ValGet        │   │ • XYPosGet/Set  │   │ • AutoApproach      │   │
│  │ • NamesGet      │   │ • ZCtrlWithdraw │   │ • ZCtrl             │   │
│  └─────────────────┘   └─────────────────┘   └─────────────────────┘   │
└──────────────────────────────┬───────────────────────────────────────────┘
                               │
┌──────────────────────────────┴───────────────────────────────────────────┐
│                           Protocol Layer                                 │
│                                                                          │
│  ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────────┐   │
│  │ Serialization   │   │ Message Headers │   │ Type Conversion     │   │
│  │ • Big-endian    │   │ • Command names │   │ • NanonisValue      │   │
│  │ • Type specs    │   │ • Body types    │   │ • Type safety       │   │
│  └─────────────────┘   └─────────────────┘   └─────────────────────┘   │
└──────────────────────────────┬───────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        Nanonis Hardware                                 │
│                         (TCP Server)                                    │
└─────────────────────────────────────────────────────────────────────────┘
```

## Data Flow

```
1. Raw Signals          2. Classification        3. Decision           4. Actions
   (f32 values)            (TipState)              (PolicyDecision)      (Hardware)
                                                                        
Nanonis ──────► Controller ──────► Classifier ──────► Policy ──────► Controller
Hardware                     │                                           │
   ▲                         ▼                                           │
   │                    TipState {                                       │
   │                      signal: 1.5,                                  │
   │                      classification: Good,                          │
   │                      position: (x, y),                             │
   │                      history: [1.4, 1.5, 1.5]                     │
   │                    }                                                │
   │                                                                     │
   └─────────────────────────────────────────────────────────────────────┘
```

## Module Dependencies

```
types.rs ◄─────────────┐
   ▲                   │
   │                   │
error.rs ◄─┐           │
   ▲       │           │
   │       │           │
protocol.rs ◄─┐        │
   ▲          │        │
   │          │        │
client.rs ◄───┼────────┤
   ▲          │        │
   │          │        │
classifier.rs ◄┘       │
   ▲                   │
   │                   │
policy.rs ◄────────────┘
   ▲
   │
controller.rs
```

## Component Responsibilities

| Component | Primary Responsibility | Key Types |
|-----------|----------------------|-----------|
| **Controller** | Orchestrate data flow, execute actions | `Controller`, `SystemStats` |
| **Classifier** | Interpret raw signals → tip states | `StateClassifier`, `BoundaryClassifier` |
| **Policy** | Make decisions from tip states | `PolicyEngine`, `RuleBasedPolicy` |
| **Client** | High-level Nanonis API | `NanonisClient`, `ConnectionConfig` |
| **Protocol** | Low-level TCP communication | `Protocol` functions |
| **Types** | Shared data structures | `TipState`, `NanonisValue`, `Position` |
| **Error** | Error handling | `NanonisError` |

## Extension Points for ML/AI

The architecture is designed for future ML/transformer expansion:

```
Current: BoundaryClassifier → RuleBasedPolicy
Future:  MLClassifier ──────→ TransformerPolicy
         │                    │
         ├─ Neural Networks   ├─ Reinforcement Learning  
         ├─ Signal Processing ├─ Decision Trees
         └─ Feature Engineering └─ Multi-agent Systems
```

## Key Design Principles

1. **Separation of Concerns**: Each module has a single, clear responsibility
2. **Dependency Inversion**: High-level modules depend on abstractions (traits)
3. **Type Safety**: Strong typing prevents runtime errors
4. **Extensibility**: Trait-based design allows easy addition of new classifiers/policies
5. **Testability**: Clean interfaces enable comprehensive unit testing