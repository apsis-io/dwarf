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

function simpleType(node) {
  if (node === undefined) return null;
  const text = node.getText().replace(/\s+/g, " ").trim();
  if (SIMPLE.has(text)) return text;
  // `readonly T[]` and `T[]` marshal identically; JSDoc has no readonly.
  const ro = /^readonly (\w+)\[\]$/.exec(text);
  if (ro !== null && SIMPLE.has(`${ro[1]}[]`)) return `${ro[1]}[]`;
  return null;
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
      // An optional or rest parameter changes the arity a caller may use,
      // which JSDoc can express but the marshalling boundary cannot; leave
      // the whole function alone rather than describe it half-truthfully.
      if (p.questionToken !== undefined || p.dotDotDotToken !== undefined) return void out.set(name, null);
      const type = simpleType(p.type);
      if (type === null || !ts.isIdentifier(p.name)) return void out.set(name, null);
      params.push({ name: p.name.text, type });
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
    const names = binding.fn.parameters.map((p) => (ts.isIdentifier(p.name) ? p.name.text : null));
    if (names.some((n) => n === null)) continue;

    const lines = ["/**"];
    sig.params.forEach((p, i) => lines.push(` * @param {${p.type}} ${names[i]}`));
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
