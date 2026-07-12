// Types for the `scule` polyfill (`--polyfill scule`) - string case
// conversion from unjs/scule. Signatures simplified to `string -> string`
// rather than scule's own literal-string-type-inferring generics (a TS
// nicety, not needed for this being genuinely useful at runtime).

declare const scule: {
  splitByCase(str: string, separators?: readonly string[]): string[];
  upperFirst(str: string): string;
  lowerFirst(str: string): string;
  isUppercase(char?: string): boolean | undefined;
  camelCase(str: string | readonly string[], opts?: { normalize?: boolean }): string;
  pascalCase(str: string | readonly string[], opts?: { normalize?: boolean }): string;
  kebabCase(str: string | readonly string[], joiner?: string): string;
  snakeCase(str: string | readonly string[]): string;
  flatCase(str: string | readonly string[]): string;
  trainCase(str: string | readonly string[], opts?: { normalize?: boolean }): string;
  titleCase(str: string | readonly string[], opts?: { normalize?: boolean }): string;
};
