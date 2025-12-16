# Tip Prep

## Tip Prep
1. Implement script for multiple different tip prep cycle testing

  1. Create Action Destroy Tip to be able to run 

2. Change behavior when freq shift is positive. Recovery only works by pulsing a lot and hard usually

3. check if bias voltage bounds are pos. to not mess up polarity

4. Reset the list of signals to make tcp channels fucking work

## Testing

1. Implement scripts to interpret experiment logs

## Logger Injection

1. Make it possible to inject Action Results without having to run an action for edge cases.

## Signal Registry

1. SignalRegistry is very badly used right now. go over and check usage

# Clean Up

1. Clean up the action driver and think about improvements in architecture

## Nanonis-rs Library

1. Publish nanonis-rs to crates.io (version 0.0.3 ready)
2. Write README.md and documentation for nanonis-rs
3. Add usage examples to nanonis-rs
