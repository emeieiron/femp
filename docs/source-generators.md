# Source generators (FSGP v1)

The Fe Source Generator Protocol is the contract between a build tool that compiles Fe and
a program that writes Fe source for it. FeMP is one such build tool; `fe_codec_gen` is one
such generator. Neither needs to know anything about the other beyond this page. The
output is ordinary Fe source, so the generated module also compiles with the plain `fe`
toolchain once it is written to disk.

The protocol is deliberately small: a command line in, Fe source out. There is no plugin
API, registration step or intermediate representation. A host passes no domain
information to a generator, and needs none back.

## Declaring a generator

```toml
[modules.shared.generate.serializers]
command = ["fe_codec_gen", "--source", "src/models.fe", "--stdout"]
output = "src/serializers.fe"
```

- `command` is the executable and its arguments. An executable with a relative path of
  more than one component is resolved against the module directory; a bare name is looked
  up on `PATH`. `femp doctor` reports a missing executable without running it.
- `output` is the destination of the generated module: a `.fe` path under `src/`, unique
  within the module, that must not exist in the authored sources.
- A module may declare several generators. Each has its own name and output.

## Invoking a generator

For each generator of each module that takes part in a command, the host:

1. starts `command` with the module directory as its working directory, standard input
   closed, and the host's environment;
2. reads standard output and standard error to the end, and waits for the process to exit;
3. on exit status 0, writes standard output, which must be UTF-8 Fe source, to `output`;
4. on any other status, discards standard output and fails the command before compiling
   anything.

FeMP runs every generator once per `femp check`, `build`, `test`, `package`, `prop` and
`prove`. It writes the output into a private, staged copy of the module, so a build never
modifies authored sources.

## Generator obligations

A conforming generator:

- **is deterministic.** The same command, inputs and generator version produce the same
  bytes. Output contains no timestamps, absolute paths, random identifiers or
  environment-dependent values.
- **writes no files.** Its only output is standard output and standard error. (A generator
  can offer a separate mode that writes files for use without a host, as `fe_codec_gen
  --out` does; that mode is outside the protocol.)
- **reports failure by its exit status.** A non-zero status means nothing it printed may be
  used. A generator never prints partial source and exits 0.
- **uses standard error only for diagnostics.** A diagnostic that refers to a place in a
  source file has the form:

  ```text
  path:line:column: severity: message
  ```

  `path` is relative to the module directory, `line` and `column` count from 1, and
  `severity` is `error` or `warning`. Other lines are free text.

## Host obligations

A conforming host:

- owns `output`. It creates, replaces and removes the generated file. It never replaces an
  authored file at that path.
- shows every diagnostic. FeMP prefixes the module's path to the `path` of each located
  diagnostic, so it resolves from the project root, and shows standard error as warnings
  when the generator succeeds.
- names the failing generator, as `generation failed for <module>.<name>`.
- keeps editor copies separate. `femp generate` writes the output into the source tree,
  with its own header, so editors and the plain `fe` toolchain can see it. Normal commands
  refresh a copy that exists and never create one. Generated paths belong in the project's
  ignore list.

## Incremental builds

FeMP reruns every generator on every command and does not cache results. A command line
does not identify the generator binary that will run (`cargo run ...` may rebuild it), so a
cache keyed on the command and its inputs could reuse stale output, and generation is not
the slow part of a build. A future version may add declared inputs for caching; until then
a generator must stay cheap enough to run each time.

## Compatibility

This page describes version 1. A change that a conforming generator or host could observe
— a new required field, a different invocation or a new obligation — makes a new version.
Hosts and generators that follow these rules interoperate without sharing code.
