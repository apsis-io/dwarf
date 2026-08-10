/* Carry the hand-written .d.ts signatures into the bundle as JSDoc.
 *
 * A polyfill bundle is plain JavaScript, so every parameter is implicitly
 * `any`. That does not matter for QuickJS, but it stops scriptc dead —
 * and not just for the polyfill itself: a USER module that imports one
 * cannot compile at all, refusing with "the reference to 'x' (a binding
 * form with no lowering)". Typing the exports unblocks the caller, which
 * then inlines the polyfill code into its own compiled body, so the
 * polyfill's own crossing cost never arises.
 *
 * TypeScript will not apply a sibling .d.ts to a .js implementation (it
 * SHADOWS it for consumers instead), so the signatures have to reach the
 * bundle as JSDoc. They are already written by hand for every polyfill;
 * this copies them rather than asking anyone to keep a second set.
 *
 * Deliberately conservative: only types that map cleanly to JSDoc are
 * emitted, and a function whose signature has anything else is left
 * exactly as it was. An unannotated function is the status quo; a wrongly
 * annotated one would be a lie the compiler believes. */
import ts from "typescript";

/** Types worth emitting. Anything outside this set is left alone —
 * `unknown`, object literals, and generics have no useful JSDoc form here
 * and would only mislead. */
const SIMPLE = new Set([
  "string",
  "number",
  "boolean",
  "void",
  "string[]",
  "number[]",
  "boolean[]",
  "Uint8Array",
]);

/** One type, in JSDoc spelling, or null if it has no faithful one.
 *
 * A union of expressible members is expressible, so this recurses rather
 * than matching whole strings — `string | readonly string[]` is common in
 * these APIs and is exactly what JSDoc's union syntax is for. */
function simpleType(node) {
  if (node === undefined) return null;
  if (ts.isUnionTypeNode(node)) {
    const members = node.types.map((t) => simpleType(t));
    if (members.some((m) => m === null)) return null;
    return [...new Set(members)].join(" | ");
  }
  if (ts.isParenthesizedTypeNode(node)) return simpleType(node.type);
  const text = node.getText().replace(/\s+/g, " ").trim();
  if (SIMPLE.has(text)) return text;
  // JSDoc has no `readonly` modifier; ReadonlyArray<T> says the same thing
  // and TypeScript accepts it in a JSDoc type position.
  const ro = /^readonly (\w+)\[\]$/.exec(text);
  if (ro !== null && SIMPLE.has(`${ro[1]}[]`)) return `ReadonlyArray<${ro[1]}>`;
  return null;
}

/** The element type a rest parameter accepts: `...paths: string[]` takes
 * strings, and JSDoc spells that `{...string}`. */
function restElementType(node) {
  if (node === undefined) return null;
  if (ts.isArrayTypeNode(node)) return simpleType(node.elementType);
  const text = node.getText().replace(/\s+/g, " ").trim();
  const m = /^(?:readonly )?(\w+)\[\]$/.exec(text);
  return m !== null && SIMPLE.has(m[1]) ? m[1] : null;
}

/**
 * Signatures from a polyfill's .d.ts, by function name.
 *
 * Both shapes the polyfills use are read: a `declare const x: { f(): T }`
 * namespace object, and bare `declare function f(): T` declarations.
 * Overloads take the first signature, since JSDoc cannot express the rest
 * and a partial annotation is worse than none.
 */
export function signaturesFromDts(dtsText, fileName = "polyfill.d.ts") {
  const sf = ts.createSourceFile(fileName, dtsText, ts.ScriptTarget.ES2022, true);
  const out = new Map();

  const record = (name, sig) => {
    if (out.has(name)) {
      out.set(name, null); // an overload: no single JSDoc describes it
      return;
    }
    const params = [];
    for (const p of sig.parameters) {
      if (!ts.isIdentifier(p.name)) return void out.set(name, null);
      // Optional and rest parameters both have a JSDoc spelling, and both
      // are worth carrying: such a function will not cross the component
      // boundary itself, but typing it is what lets a CALLER compile.
      const rest = p.dotDotDotToken !== undefined;
      const type = rest ? restElementType(p.type) : simpleType(p.type);
      if (type === null) return void out.set(name, null);
      params.push({
        name: p.name.text,
        type,
        rest,
        optional: p.questionToken !== undefined || p.initializer !== undefined,
      });
    }
    const returns = simpleType(sig.type);
    if (returns === null) return void out.set(name, null);
    out.set(name, { params, returns });
  };

  const visit = (node) => {
    if (ts.isMethodSignature(node) && ts.isIdentifier(node.name)) record(node.name.text, node);
    else if (ts.isFunctionDeclaration(node) && node.name !== undefined) record(node.name.text, node);
    ts.forEachChild(node, visit);
  };
  visit(sf);

  for (const [name, sig] of out) if (sig === null) out.delete(name);
  return out;
}

/**
 * Insert a JSDoc block above each bundled function the .d.ts describes.
 *
 * Both top-level forms a bundler emits are matched: `function f(...)`
 * (esbuild's usual output) and `var f = function(...)` / `= (...) =>`,
 * which is what pathe's bundle produces. A name declared some other way is
 * skipped — there is nothing to annotate, and the export keeps working
 * untyped.
 */
/** Does this binding already carry a JSDoc block with types? */
function hasTypeComment(code, fn, pos) {
  // The comment sits above the STATEMENT, so scan from its full start —
  // for `var f = function(){}` the function expression's own leading
  // trivia is empty and would miss it.
  const ranges = ts.getLeadingCommentRanges(code, Math.max(0, fn.pos)) ?? [];
  if (ranges.some((r) => /@param|@returns/.test(code.slice(r.pos, r.end)))) return true;
  // `var f = function(){}` puts the comment above the STATEMENT, where the
  // function expression's own leading trivia cannot see it.
  const before = code.slice(Math.max(0, pos - 512), pos);
  const lastBlock = before.lastIndexOf("/**");
  return lastBlock !== -1 && /^[\s]*$/.test(before.slice(before.indexOf("*/", lastBlock) + 2)) &&
    /@param|@returns/.test(before.slice(lastBlock));
}

export function annotateBundle(code, signatures) {
  const sf = ts.createSourceFile("bundle.js", code, ts.ScriptTarget.ES2022, true);
  const edits = [];

  /** The function-ish node a top-level statement binds, with the name it
   * binds it under and where a JSDoc block for it would go. */
  const bindings = [];
  for (const stmt of sf.statements) {
    if (ts.isFunctionDeclaration(stmt) && stmt.name !== undefined) {
      bindings.push({ name: stmt.name.text, fn: stmt, pos: stmt.getStart(sf) });
      continue;
    }
    if (!ts.isVariableStatement(stmt)) continue;
    // One declaration only: a JSDoc block above `var a = ..., b = ...`
    // would claim to describe the whole statement.
    const decls = stmt.declarationList.declarations;
    if (decls.length !== 1) continue;
    const [decl] = decls;
    if (!ts.isIdentifier(decl.name) || decl.initializer === undefined) continue;
    if (!ts.isFunctionExpression(decl.initializer) && !ts.isArrowFunction(decl.initializer)) continue;
    bindings.push({ name: decl.name.text, fn: decl.initializer, pos: stmt.getStart(sf) });
  }

  for (const binding of bindings) {
    const sig = signatures.get(binding.name);
    if (sig === undefined) continue;
    // Already annotated: re-running must not stack a second block on top
    // of the first (the bundles are regenerated and re-annotated, so this
    // pass has to be safe to repeat).
    if (hasTypeComment(code, binding.fn, binding.pos)) continue;
    // Only annotate when the bundle's own parameters match the .d.ts
    // arity — a mismatch means the two have drifted, and guessing which
    // parameter is which is exactly the lie to avoid.
    if (binding.fn.parameters.length !== sig.params.length) continue;
    const params = binding.fn.parameters;
    const names = params.map((p) => (ts.isIdentifier(p.name) ? p.name.text : null));
    if (names.some((n) => n === null)) continue;
    // ...and when they agree on WHICH parameter is the rest one. A rest
    // marker on the wrong side describes a different function.
    if (params.some((p, i) => (p.dotDotDotToken !== undefined) !== sig.params[i].rest)) continue;

    const lines = ["/**"];
    sig.params.forEach((p, i) => {
      // A default value in the bundle makes a parameter optional whatever
      // the .d.ts says, and JSDoc marks that with brackets.
      const optional = p.optional || params[i].initializer !== undefined;
      const type = p.rest ? `...${p.type}` : p.type;
      lines.push(` * @param {${type}} ${optional && !p.rest ? `[${names[i]}]` : names[i]}`);
    });
    lines.push(` * @returns {${sig.returns}}`, " */", "");
    edits.push({ pos: binding.pos, text: lines.join("\n") });
  }

  // Apply back to front so earlier offsets stay valid.
  let out = code;
  for (const edit of edits.sort((a, b) => b.pos - a.pos)) {
    out = out.slice(0, edit.pos) + edit.text + out.slice(edit.pos);
  }
  return { code: out, annotated: edits.length };
}
