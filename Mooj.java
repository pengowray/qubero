import java.io.*;
import javax.swing.JLabel;

public class Mooj {
    private SimpleFileChunk root;

    public static void main(String[] arg) {
        // read in file
        if (arg.length == 0)
            arg = new String[] {"C:\\My Documents\\project-moojasm\\HexEditorGUI.class"}; // Mooj.class

        new Mooj( arg[0] ).start();
    }

    public Mooj(String filename) {
        root = new SimpleFileChunk(filename);
        
    }

    protected void start() {
        new HexEditorGUI(root);
    }

    //XXX: remove this:
    public String getRootName() {
        return root.toString();
    }

    //XXX: remove this:
    public Chunk getRootChunk() {
        return root;
    }
}



/** 
 * A view contains information on how to display chunks, options and commands
 * in a USEFUL manner -- hiding redundancy and triviality. 
 *
 * XXX: In future, there should be only a handful of generic views, which will be
 * user-customisable and dynamic. e.g. "zoom" level of detail. Perhaps file format specific
 * views will become necessary also.
 */
class View {
    
}

/**
 * This presentation displays only hexidecimal values.
 */
class PlainHexView extends View {
    
}

/**
 * This presentation favours displaying hexidecimal values over "friendlier" formats.
 */
class HexView {
    
}

/**
 * This presentation favours displaying high level information over lower level (eg hex) formats.
 */
class HighView {
    
}

/**
 * Display multiple levels: formatted for printing.
 */
class PrintView {
    
}

/**
 * In the debug view, everything is shown.
 */
class DebugView extends View {
    
}

/**
 * Provides the rendering engines which do the dirty work.
class RendererFactory {
    
}
 */
