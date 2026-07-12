// Types for the `knitwork` polyfill (`--polyfill knitwork`) - JS/TS
// code-STRING generation utilities from unjs/knitwork (import/export
// statements, object/array literals, TypeScript interfaces, as strings -
// no parsing of existing code, unlike e.g. magicast, which also needs
// Node's `fs` and a full AST parser and so isn't a good fit for a WASM
// sandbox). Useful for a component that emits JS/TS source dynamically
// (codegen, scaffolding, config generation).

interface CodegenOptions {
  singleQuotes?: boolean;
}
type ESMImport = string | { name: string; as?: string };
type ESMExport = string | { name: string; as?: string };
interface ESMCodeGenOptions extends CodegenOptions {
  attributes?: { type: string };
}
interface GenObjectOptions extends CodegenOptions {
  interopDefault?: boolean;
}
interface GenInterfaceOptions {
  export?: boolean;
  extends?: string | string[];
  comment?: string;
}
type TypeObject = Record<string, string | [string, string]>;

declare const knitwork: {
  genImport(specifier: string, imports?: ESMImport | ESMImport[], options?: ESMCodeGenOptions): string;
  genTypeImport(specifier: string, imports: ESMImport[], options?: ESMCodeGenOptions): string;
  genExport(specifier: string, exports?: ESMExport | ESMExport[], options?: ESMCodeGenOptions): string;
  genTypeExport(specifier: string, imports: ESMImport[], options?: ESMCodeGenOptions): string;
  genDynamicImport(specifier: string, options?: ESMCodeGenOptions & { comment?: string; wrapper?: boolean; interopDefault?: boolean }): string;
  genDynamicTypeImport(specifier: string, name: string | undefined, options?: ESMCodeGenOptions): string;
  genInlineTypeImport(specifier: string, name?: string, options?: ESMCodeGenOptions): string;

  genObjectFromRaw(object: Record<string, unknown>, indent?: string, options?: GenObjectOptions): string;
  genObjectFromValues(obj: Record<string, unknown>, indent?: string, options?: GenObjectOptions): string;
  genObjectFromRawEntries(array: [key: string, value: unknown][], indent?: string, options?: GenObjectOptions): string;
  genArrayFromRaw(array: unknown[], indent?: string, options?: GenObjectOptions): string;
  genObjectKey(key: string): string;

  genString(input: string, options?: CodegenOptions): string;
  escapeString(id: string): string;
  genSafeVariableName(name: string): string;
  wrapInDelimiters(lines: string[], indent?: string, delimiters?: string, withComma?: boolean): string;

  genTypeObject(object: TypeObject, indent?: string): string;
  genInterface(name: string, contents?: TypeObject, options?: GenInterfaceOptions, indent?: string): string;
  genAugmentation(specifier: string, interfaces?: Record<string, TypeObject | [TypeObject, Omit<GenInterfaceOptions, "export">]>): string;
};
