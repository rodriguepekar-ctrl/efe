// Tutorial 1 — Classify a single text via the Java SDK.
//
// Sprint Φ-C-SDK Completion (D32) — artefact #25.
//
// Compile :
//   javac --release 21 Main.java
//
// Run :
//   EFE_HOST=efe.example.com:443 java Main

// Once the Java SDK is published, this becomes :
// import ai.efe.EfeClient;
// import ai.efe.Verdict;

public final class Main {
    public static void main(String[] args) {
        var host = System.getenv().getOrDefault("EFE_HOST", "efe.example.com:443");

        // Post-Φ-C-SDK D31 :
        //
        //   try (EfeClient client = EfeClient.builder()
        //           .host(host)
        //           .build()) {
        //       Verdict verdict = client.classify(
        //           "I want to dispute the charge on my account",
        //           "router-v1");
        //       System.out.printf("label       : %s%n", verdict.label());
        //       System.out.printf("confidence  : %.4f%n", verdict.confidence());
        //       System.out.printf("rounds      : %d%n", verdict.roundsExecuted());
        //       System.out.printf("swarm_id    : %s%n", verdict.swarmId());
        //       System.out.printf("catalog_ver : %s%n", verdict.catalogVersion());
        //   } catch (Exception e) {
        //       e.printStackTrace();
        //       System.exit(1);
        //   }

        System.out.printf("tutorial-1 (Java) connecting to %s%n", host);
        System.out.println(
            "Once Φ-C-SDK D31 wires the Java SDK grpc-java stubs, this tutorial"
        );
        System.out.println(
            "will perform an end-to-end classify call. For now it documents the"
        );
        System.out.println(
            "idiomatic fluent-builder API shape per docs/sdk-auth-conventions.md."
        );
    }
}
