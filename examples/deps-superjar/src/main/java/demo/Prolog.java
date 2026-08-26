package demo;

import com.dylibso.chicory.runtime.Store;
import com.dylibso.chicory.wasi.WasiExitException;
import com.dylibso.chicory.wasi.WasiOptions;
import com.dylibso.chicory.wasi.WasiPreview1;
import com.dylibso.chicory.wasm.Parser;
import com.dylibso.chicory.wasm.WasmModule;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * Drives the compiled-Prolog WASI module embedded as a classpath resource.
 *
 * <p>Each query instantiates the module fresh — the process-per-request model
 * of the native binary, minus the process. The module speaks the exact wire
 * contract of the native compiled binary: same flags, same text output, same
 * exit codes.
 */
public final class Prolog {

    /** The compiled binary's wire contract, surfaced to Java. */
    public record Result(int exitCode, String stdout, String stderr) {
        /** Exit 1: at least one solution was found. */
        public boolean hasSolutions() {
            return exitCode == 1;
        }

        /** Exit 0: the query is consistent but has no solutions. */
        public boolean noSolutions() {
            return exitCode == 0;
        }
    }

    private final WasmModule module;
    private final String programName;

    private Prolog(WasmModule module, String programName) {
        this.module = module;
        this.programName = programName;
    }

    /** Parse the Wasm module once; instantiation happens per query. */
    public static Prolog fromResource(String resourcePath, String programName) {
        byte[] bytes;
        try (InputStream in = Prolog.class.getResourceAsStream(resourcePath)) {
            if (in == null) {
                throw new IllegalStateException(
                        "missing resource "
                                + resourcePath
                                + " — was the jar built with `just deps-superjar`?");
            }
            bytes = in.readAllBytes();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return new Prolog(Parser.parse(bytes), programName);
    }

    /** {@code --query goal} with the compiled-in defaults. */
    public Result query(String goal) {
        return run(List.of("--query", goal));
    }

    /** {@code --query goal --limit n}. */
    public Result query(String goal, int limit) {
        return run(List.of("--query", goal, "--limit", Integer.toString(limit)));
    }

    /**
     * Run the module with the same argv the native binary accepts, capturing
     * stdout/stderr and the exit code. {@code env} is virtualized WASI state
     * (e.g. {@code PLG_MAX_STEPS}); the guest never sees the host environment.
     */
    public Result run(List<String> args, Map<String, String> env) {
        var stdout = new ByteArrayOutputStream();
        var stderr = new ByteArrayOutputStream();
        var argv = new ArrayList<String>();
        argv.add(programName);
        argv.addAll(args);

        var options = WasiOptions.builder();
        options.withStdout(stdout).withStderr(stderr).withArguments(argv);
        env.forEach(options::withEnvironment);

        var wasi = WasiPreview1.builder().withOptions(options.build()).build();
        var store = new Store().addFunction(wasi.toHostFunctions());
        int exitCode = 0;
        try {
            // Instantiating a WASI command module runs `_start` (the spec's
            // command pattern); the query executes here.
            store.instantiate(programName, module);
        } catch (WasiExitException e) {
            // The program always ends in proc_exit; Chicory reports the code
            // this way rather than as a return value.
            exitCode = e.exitCode();
        }
        return new Result(
                exitCode, utf8(stdout), utf8(stderr));
    }

    public Result run(List<String> args) {
        return run(args, Map.of());
    }

    private static String utf8(ByteArrayOutputStream out) {
        return out.toString(StandardCharsets.UTF_8);
    }
}
