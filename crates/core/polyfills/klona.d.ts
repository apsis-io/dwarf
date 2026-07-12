// Types for the `klona` polyfill (`--polyfill klona`) - a fast deep-clone
// utility from unjs/klona. dwarf's QuickJS-ng has no `structuredClone` at
// all (confirmed, not just "klona is faster") - this fills that gap.
// Handles plain objects/arrays, Map, Set, Date, RegExp, ArrayBuffer/typed
// arrays/DataView - same coverage as structuredClone's common cases, though
// not a spec-exact structuredClone replacement (e.g. no cross-realm/
// transferable-object semantics, no cycle detection).

declare function klona<T>(input: T): T;
