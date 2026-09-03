import com.github.gumtreediff.gen.jdt.JdtTreeGenerator;
import com.github.gumtreediff.tree.Tree;
import com.github.gumtreediff.tree.TreeContext;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public final class EnumerateJdt {
    private EnumerateJdt() {}

    public static void main(String[] args) throws Exception {
        if (args.length == 0) {
            throw new IllegalArgumentException("expected at least one Java source path");
        }

        PrintWriter output = new PrintWriter(System.out, false, StandardCharsets.UTF_8);
        for (int index = 0; index < args.length; index++) {
            String source = Files.readString(Path.of(args[index]), StandardCharsets.UTF_8);
            TreeContext context = new JdtTreeGenerator().generateFrom().string(source);
            output.printf("BEGIN\t%d%n", index);
            for (Tree node : context.getRoot().preOrder()) {
                String type = node.getType().name;
                if (type.indexOf('\t') >= 0 || type.indexOf('\n') >= 0 || type.indexOf('\r') >= 0) {
                    throw new IllegalStateException("JDT node type is not TSV-safe");
                }
                output.printf("NODE\t%s\t%d\t%d%n", type, node.getPos(), node.getEndPos());
            }
            output.printf("END\t%d%n", index);
        }
        output.flush();
        if (output.checkError()) {
            throw new IllegalStateException("failed to write JDT enumeration output");
        }
    }
}
