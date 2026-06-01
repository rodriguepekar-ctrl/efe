// Sprint **Φ-C-SDK Completion (D30)** — Java SDK contract runner.
//
// Cf. docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md § 5 + artefact #14.
//
// Reads /fixtures/quartet_fixtures.json, calls the Java SDK gRPC client
// for each fixture, writes verdicts to /shared/out_java.json.
//
// Mock mode (EFE_CONTRACT_MOCK=1) replicates the D25 Φ-C-FFI formula
// bit-identical to the Python + Go + TS runners.
//
// Build :
//   javac --release 21 Main.java
//
// Run :
//   EFE_CONTRACT_MOCK=1 java -cp . Main
//
// Or via the contract-runner Docker image (= JDK 21 + this single file).

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;

/**
 * Java SDK contract runner — emits a JSON file of verdicts for the
 * 100 quartet fixtures, used by sdk/contract-runner/compare.py to
 * verify bit-identity across SDKs.
 *
 * <p>Pure JDK : zero third-party dependencies. JSON output is built
 * with manual StringBuilder serialisation (= avoids dragging in
 * Jackson/Gson just for one tool).
 *
 * <p>Verdict computation in mock mode is bit-identical to the
 * Rust efe-ffi D25 stub : SHA-256 on f32 little-endian bytes of the
 * synthetic embedding, then label index from digest[0..4] mod
 * labels.length, confidence from digest[4..8] / u32::MAX, swarm_id
 * = hex(digest[0..8]).
 */
public final class Main {

    private static final List<String> DEFAULT_LABELS = List.of(
        "billing-dispute",
        "tech-support",
        "refund-request",
        "general-inquiry",
        "other"
    );
    private static final String DEFAULT_CATALOG_VERSION = "1.0.0";
    private static final String DEFAULT_BUNDLE_ID = "test-bundle";
    private static final int DIM = 384;

    /**
     * Replicate D25 stub : (byte / 255 - 0.5) → L2 normalize → f32 LE bytes.
     *
     * <p>Java {@code float} is IEEE 754 single-precision (= matches Rust
     * f32). All intermediate arithmetic is in {@code float}, NOT
     * {@code double}, to preserve cross-language bit-identity.
     */
    static byte[] encodeText(String text, String bundleId) throws NoSuchAlgorithmException {
        var seedHasher = MessageDigest.getInstance("SHA-256");
        seedHasher.update(text.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        seedHasher.update(bundleId.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        var seed = seedHasher.digest(); // 32 bytes

        var emb = new float[DIM];
        for (int i = 0; i < DIM; i++) {
            int b = seed[i % 32] & 0xff; // unsigned byte (= match Rust u8)
            emb[i] = (float) b / 255.0f - 0.5f;
        }
        // Linear f32 sum-of-squares (= match Rust .sum::<f32>()).
        float sumSq = 0.0f;
        for (float v : emb) {
            sumSq += v * v;
        }
        float norm = (float) Math.sqrt((double) sumSq);
        if (norm > 0.0f) {
            for (int i = 0; i < DIM; i++) {
                emb[i] = emb[i] / norm;
            }
        }
        // Pack as little-endian f32 bytes (= match Rust to_le_bytes()).
        var buf = ByteBuffer.allocate(DIM * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (float v : emb) {
            buf.putFloat(v);
        }
        return buf.array();
    }

    /**
     * Bit-identical replica of efe-ffi controller::synthesise_verdict.
     */
    static Verdict mockVerdict(Fixture f) throws NoSuchAlgorithmException {
        var embBytes = encodeText(f.text, DEFAULT_BUNDLE_ID);

        var h = MessageDigest.getInstance("SHA-256");
        h.update(embBytes);
        h.update(f.catalogId.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        // seed as u64 little-endian.
        var seedBuf = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN);
        seedBuf.putLong(f.seed);
        h.update(seedBuf.array());
        var digest = h.digest();

        // Read u32 LE from digest[0..4] for label_idx.
        var d04 = ByteBuffer.wrap(digest, 0, 4).order(ByteOrder.LITTLE_ENDIAN);
        long labelIdxRaw = Integer.toUnsignedLong(d04.getInt());
        int labelIdx = (int) (labelIdxRaw % DEFAULT_LABELS.size());
        var label = DEFAULT_LABELS.get(labelIdx);

        // u32 LE from digest[4..8].
        var d48 = ByteBuffer.wrap(digest, 4, 4).order(ByteOrder.LITTLE_ENDIAN);
        long confU32 = Integer.toUnsignedLong(d48.getInt());
        // Match Rust : (conf_u32 as f32) / (u32::MAX as f32).
        float confidenceF32 = (float) confU32 / (float) (0xFFFFFFFFL);
        // u32::MAX as f32 rounds to 4294967296.0f (= 2^32) ; same as Rust.

        int rounds = 4 + ((digest[8] & 0xff) % 16);
        var swarmHex = HexFormat.of().formatHex(java.util.Arrays.copyOfRange(digest, 0, 8));

        var v = new Verdict();
        v.fixtureId = f.id;
        v.label = label;
        v.confidence = (double) confidenceF32; // f32 → f64 exact cast
        v.coneId = "cone-" + f.catalogId;
        v.roundsExecuted = rounds;
        v.swarmId = "swarm-" + swarmHex;
        v.catalogVersion = DEFAULT_CATALOG_VERSION;
        v.subScores = new ArrayList<>();
        v.latencyMs = 0.0;
        return v;
    }

    static List<Fixture> parseFixtures(String json) {
        var fixtures = new ArrayList<Fixture>();
        // Minimal JSON parser : look for `"fixtures": [...]` array.
        //
        // **Audit fix Φ-C-SDK.B14** — pre-fix the brace-counting did
        // NOT track string state. A fixture text containing `{` or `}`
        // (e.g. "Brackets: () [] {} <>") would silently break the
        // depth counter → premature object close → wrong verdicts or
        // crash. Post-fix : skip braces inside JSON strings,
        // handling `\"` escape.
        int arrStart = json.indexOf("\"fixtures\"");
        if (arrStart < 0) return fixtures;
        int bracketStart = json.indexOf('[', arrStart);
        int depth = 0;
        int objStart = -1;
        boolean inString = false;
        boolean escape = false;
        for (int i = bracketStart; i < json.length(); i++) {
            char c = json.charAt(i);
            // String state machine : skip everything inside `"..."`.
            if (inString) {
                if (escape) {
                    escape = false;
                } else if (c == '\\') {
                    escape = true;
                } else if (c == '"') {
                    inString = false;
                }
                continue; // do NOT process braces while in string
            }
            if (c == '"') {
                inString = true;
                continue;
            }
            // Now we're at a structural char.
            if (c == '{') {
                if (depth == 0) objStart = i;
                depth++;
            } else if (c == '}') {
                depth--;
                if (depth == 0 && objStart >= 0) {
                    var objText = json.substring(objStart, i + 1);
                    fixtures.add(parseFixtureObject(objText));
                    objStart = -1;
                }
            } else if (c == ']' && depth == 0) {
                break;
            }
        }
        return fixtures;
    }

    static Fixture parseFixtureObject(String obj) {
        var f = new Fixture();
        f.id = extractString(obj, "id");
        f.text = extractString(obj, "text");
        f.catalogId = extractString(obj, "catalog_id");
        var seedStr = extractValue(obj, "seed");
        f.seed = Long.parseLong(seedStr.trim());
        return f;
    }

    static String extractString(String obj, String key) {
        int keyPos = obj.indexOf("\"" + key + "\"");
        if (keyPos < 0) return "";
        int colonPos = obj.indexOf(':', keyPos);
        int quoteStart = obj.indexOf('"', colonPos);
        // Find end-quote, handling \" escapes.
        var sb = new StringBuilder();
        for (int i = quoteStart + 1; i < obj.length(); i++) {
            char c = obj.charAt(i);
            if (c == '\\' && i + 1 < obj.length()) {
                char esc = obj.charAt(i + 1);
                switch (esc) {
                    case 'n' -> sb.append('\n');
                    case 't' -> sb.append('\t');
                    case 'r' -> sb.append('\r');
                    case '"' -> sb.append('"');
                    case '\\' -> sb.append('\\');
                    case '/' -> sb.append('/');
                    case 'u' -> {
                        var hex = obj.substring(i + 2, i + 6);
                        sb.append((char) Integer.parseInt(hex, 16));
                        i += 4;
                    }
                    default -> sb.append(esc);
                }
                i++;
            } else if (c == '"') {
                return sb.toString();
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    static String extractValue(String obj, String key) {
        int keyPos = obj.indexOf("\"" + key + "\"");
        if (keyPos < 0) return "";
        int colonPos = obj.indexOf(':', keyPos);
        var sb = new StringBuilder();
        boolean started = false;
        for (int i = colonPos + 1; i < obj.length(); i++) {
            char c = obj.charAt(i);
            if (!started && (c == ' ' || c == '\t')) continue;
            if (c == ',' || c == '}') break;
            started = true;
            sb.append(c);
        }
        return sb.toString();
    }

    /**
     * Serialise the output payload as JSON. Manual to avoid
     * external dependencies.
     */
    static String serialiseOutput(String version, List<Verdict> verdicts) {
        var sb = new StringBuilder();
        sb.append("{\n");
        sb.append("  \"sdk\": \"java\",\n");
        sb.append("  \"version\": \"").append(escapeJson(version)).append("\",\n");
        sb.append("  \"count\": ").append(verdicts.size()).append(",\n");
        sb.append("  \"verdicts\": [\n");
        for (int i = 0; i < verdicts.size(); i++) {
            sb.append("    ");
            sb.append(serialiseVerdict(verdicts.get(i)));
            if (i < verdicts.size() - 1) sb.append(",");
            sb.append("\n");
        }
        sb.append("  ]\n");
        sb.append("}\n");
        return sb.toString();
    }

    static String serialiseVerdict(Verdict v) {
        var sb = new StringBuilder();
        sb.append("{");
        sb.append("\"fixture_id\":\"").append(escapeJson(v.fixtureId)).append("\",");
        sb.append("\"label\":\"").append(escapeJson(v.label)).append("\",");
        sb.append("\"confidence\":").append(v.confidence).append(",");
        sb.append("\"cone_id\":\"").append(escapeJson(v.coneId)).append("\",");
        sb.append("\"rounds_executed\":").append(v.roundsExecuted).append(",");
        sb.append("\"swarm_id\":\"").append(escapeJson(v.swarmId)).append("\",");
        sb.append("\"catalog_version\":\"").append(escapeJson(v.catalogVersion)).append("\",");
        sb.append("\"sub_scores\":[],");
        sb.append("\"latency_ms\":").append(v.latencyMs);
        sb.append("}");
        return sb.toString();
    }

    static String escapeJson(String s) {
        var sb = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> sb.append("\\\"");
                case '\\' -> sb.append("\\\\");
                case '\n' -> sb.append("\\n");
                case '\r' -> sb.append("\\r");
                case '\t' -> sb.append("\\t");
                default -> sb.append(c);
            }
        }
        return sb.toString();
    }

    public static void main(String[] args) throws IOException, NoSuchAlgorithmException {
        var fixturesPath = System.getenv()
            .getOrDefault("FIXTURES_PATH", "/fixtures/quartet_fixtures.json");
        var outPath = System.getenv()
            .getOrDefault("OUT_PATH", "/shared/out_java.json");
        var versionLabel = System.getenv().getOrDefault("SDK_VERSION", "0.6.0");

        if (!"1".equals(System.getenv("EFE_CONTRACT_MOCK"))) {
            System.err.println(
                "FATAL : live gRPC path not yet wired ; set EFE_CONTRACT_MOCK=1 to use mock"
            );
            System.exit(2);
        }

        var json = Files.readString(Paths.get(fixturesPath));
        var fixtures = parseFixtures(json);
        var verdicts = new ArrayList<Verdict>(fixtures.size());
        for (var f : fixtures) {
            verdicts.add(mockVerdict(f));
        }

        var out = serialiseOutput(versionLabel, verdicts);
        Path outFile = Paths.get(outPath);
        if (outFile.getParent() != null) {
            Files.createDirectories(outFile.getParent());
        }
        Files.writeString(outFile, out);
        System.err.printf("runner-java : wrote %d verdicts → %s%n",
            verdicts.size(), outPath);
    }

    /** Plain DTO matching the input fixture schema. */
    static final class Fixture {
        String id;
        String text;
        String catalogId;
        long seed;
    }

    /** Plain DTO matching the output verdict schema consumed by compare.py. */
    static final class Verdict {
        String fixtureId;
        String label;
        double confidence;
        String coneId;
        int roundsExecuted;
        String swarmId;
        String catalogVersion;
        List<Object> subScores;
        double latencyMs;
    }
}
