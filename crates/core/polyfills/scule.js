// scule.js - string case conversion from unjs/scule (see NOTICES)
// node_modules/scule/dist/index.mjs
var NUMBER_CHAR_RE = /\d/;
var STR_SPLITTERS = ["-", "_", "/", "."];
function isUppercase(char = "") {
  if (NUMBER_CHAR_RE.test(char)) {
    return void 0;
  }
  return char !== char.toLowerCase();
}
/**
 * @param {string} str
 * @param {ReadonlyArray<string>} [separators]
 * @returns {string[]}
 */
function splitByCase(str, separators) {
  const splitters = separators ?? STR_SPLITTERS;
  const parts = [];
  if (!str || typeof str !== "string") {
    return parts;
  }
  let buff = "";
  let previousUpper;
  let previousSplitter;
  for (const char of str) {
    const isSplitter = splitters.includes(char);
    if (isSplitter === true) {
      parts.push(buff);
      buff = "";
      previousUpper = void 0;
      continue;
    }
    const isUpper = isUppercase(char);
    if (previousSplitter === false) {
      if (previousUpper === false && isUpper === true) {
        parts.push(buff);
        buff = char;
        previousUpper = isUpper;
        continue;
      }
      if (previousUpper === true && isUpper === false && buff.length > 1) {
        const lastChar = buff.at(-1);
        parts.push(buff.slice(0, Math.max(0, buff.length - 1)));
        buff = lastChar + char;
        previousUpper = isUpper;
        continue;
      }
    }
    buff += char;
    previousUpper = isUpper;
    previousSplitter = isSplitter;
  }
  parts.push(buff);
  return parts;
}
/**
 * @param {string} str
 * @returns {string}
 */
function upperFirst(str) {
  return str ? str[0].toUpperCase() + str.slice(1) : "";
}
/**
 * @param {string} str
 * @returns {string}
 */
function lowerFirst(str) {
  return str ? str[0].toLowerCase() + str.slice(1) : "";
}
function pascalCase(str, opts) {
  return str ? (Array.isArray(str) ? str : splitByCase(str)).map((p) => upperFirst(opts?.normalize ? p.toLowerCase() : p)).join("") : "";
}
function camelCase(str, opts) {
  return lowerFirst(pascalCase(str || "", opts));
}
/**
 * @param {string | ReadonlyArray<string>} str
 * @param {string} [joiner]
 * @returns {string}
 */
function kebabCase(str, joiner) {
  return str ? (Array.isArray(str) ? str : splitByCase(str)).map((p) => p.toLowerCase()).join(joiner ?? "-") : "";
}
/**
 * @param {string | ReadonlyArray<string>} str
 * @returns {string}
 */
function snakeCase(str) {
  return kebabCase(str || "", "_");
}
/**
 * @param {string | ReadonlyArray<string>} str
 * @returns {string}
 */
function flatCase(str) {
  return kebabCase(str || "", "");
}
function trainCase(str, opts) {
  return (Array.isArray(str) ? str : splitByCase(str)).filter(Boolean).map((p) => upperFirst(opts?.normalize ? p.toLowerCase() : p)).join("-");
}
var titleCaseExceptions = /^(a|an|and|as|at|but|by|for|if|in|is|nor|of|on|or|the|to|with)$/i;
function titleCase(str, opts) {
  return (Array.isArray(str) ? str : splitByCase(str)).filter(Boolean).map(
    (p) => titleCaseExceptions.test(p) ? p.toLowerCase() : upperFirst(opts?.normalize ? p.toLowerCase() : p)
  ).join(" ");
}
export {
  camelCase,
  flatCase,
  isUppercase,
  kebabCase,
  lowerFirst,
  pascalCase,
  snakeCase,
  splitByCase,
  titleCase,
  trainCase,
  upperFirst
};
