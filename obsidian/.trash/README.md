# rusty-tip Architecture Documentation

**Description**: Obsidian documentation for rusty-tip SPM control library architecture.

**Implementation**: 
```
Raw Signals → StateClassifier → MachineState → PolicyEngine → Actions
     ↑              ↑               ↑             ↑          ↑
NanonisClient  BoundaryClassifier   Hub    RuleBasedPolicy Controller
```

**Components**:

**Traits**: [[StateClassifier]] | [[PolicyEngine]] | [[LearningPolicyEngine]] | [[ExplainablePolicyEngine]] | [[DiskWriter]]

**Structs**: [[MachineState]] | [[Controller]] | [[NanonisClient]] | [[BoundaryClassifier]] | [[RuleBasedPolicy]] | [[SessionMetadata]] | [[SyncSignalMonitor]]

**Enums**: [[TipState]] | [[PolicyDecision]] | [[ActionType]] | [[NanonisError]] | [[NanonisValue]]

**Notes**: 
- All files use [[component]] linking for Obsidian canvas visualization
- Each component documented with Description, Implementation, Notes format
- Designed for quick scanning and system diagram creation