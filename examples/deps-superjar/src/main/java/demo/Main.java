package demo;

import java.util.List;
import java.util.TreeSet;

/**
 * The demo. Two modes:
 *
 * <p>{@code java -jar deps-superjar.jar --query "needs(app, X)" [--limit N]}
 * is a pure pass-through: the jar behaves byte-for-byte like the native
 * compiled binary, exit codes included (0 no solutions, 1 solutions,
 * 2 usage error, 3 runtime error).
 *
 * <p>{@code java -jar deps-superjar.jar} (no args) runs the integration
 * showcase: Java asks the dependency-graph questions, Prolog answers them,
 * Java owns the reporting.
 */
public final class Main {

    public static void main(String[] args) {
        var prolog = Prolog.fromResource("/deps.wasm", "deps");
        if (args.length == 0) {
            showcase(prolog);
            return;
        }
        var result = prolog.run(List.of(args));
        System.out.print(result.stdout());
        System.err.print(result.stderr());
        System.exit(result.exitCode());
    }

    /**
     * Java owns the questions and the report; Prolog owns the inference.
     * The domain is examples/deps.pl: a build dependency graph with
     * transitive closure (needs/2) and shared-dependency detection.
     */
    static void showcase(Prolog prolog) {
        System.out.println("== deps-superjar: Prolog inference inside the JVM ==\n");

        // Q1: what does `app` transitively need? (recursive Prolog rule)
        var needs = prolog.query("needs(app, X)");
        var components = new TreeSet<>(bindings(needs.stdout(), "X"));
        System.out.println("app ships with: " + String.join(", ", components));

        // Q2: which components are single points of failure? Every row of
        // shares_dep(A, B, D) is a D that two distinct components need.
        var shared = prolog.query("shares_dep(A, B, D)");
        var critical = new TreeSet<>(bindings(shared.stdout(), "D"));
        System.out.println("\ncritical components (shared by 2+ dependents):");
        critical.forEach(dep -> System.out.println("  " + dep));

        // Q3: a yes/no gate Java can branch on.
        var gate = prolog.query("shares_dep(auth, render)");
        System.out.println(
                "\nauth and render share a dependency: "
                        + (gate.hasSolutions() ? "YES — coordinate their releases" : "no"));

        // Q4: the wire contract's error path, observed from Java.
        var bogus = prolog.query("nonexistent_predicate(x)");
        System.out.println(
                "\nunknown predicate → exit "
                        + bogus.exitCode()
                        + " (runtime error); the wire text says: "
                        + bogus.stdout().strip());
    }

    /** Project the text wire format (`Name = value` lines) to one variable. */
    static List<String> bindings(String stdout, String var) {
        var prefix = var + " = ";
        return stdout.lines()
                .filter(line -> line.startsWith(prefix))
                .map(line -> line.substring(prefix.length()))
                .toList();
    }

    private Main() {}
}
