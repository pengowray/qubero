/*
 * SimpleLayer.java
 *
 * A layer has: a mask, data, and resources
 *
 * Created on 25 August 2002, 08:31
 */

/**
 *
 * @author  administrator
 */
package net.pengo.layer;
import net.pengo.app.*;

abstract public class SimpleLayer {
    OpenFile openFile;
    
    // has BOUNDS == selection, start + end, or a rough box, etc.
    // has MASK, which must be within bounds, allows holes
    // BOUNDS + MASK make up the OPAQUE AREA. The OPAQUE AREA is all that user sees.
    // a SELECTION is a MASK with no RAW DATA
    
    // has RAW DATA
    // has FORUMLAE (e.g. semi-transparency)
    // ? has REACH (where the formula may draw from?)
    
    // layers may represent:
    //  - 1. a file or chunk of memory only (modify directly)
    //  - 1. a file, 2. unsaved changes.
    //  - 1. a file, 2, its memory chace, 3 undoable modifications
    //  - CVS (file over time)
    // a selection (interface a MASK)
    
    //FIXME: allow relative layer depth. e.g. "this is higher than that, and that's higher than that.."
    
    //FIXME: allow more complex bounds, eg. 2D areas. (3d areas? trees?)
    
    private int leftBound;
    private int rightBound;
    /*
    private Mask mask;
    
    public SimpleLayer(OpenFile openFile, Mask mask) {
        
    }
    */
    
    /** Creates a new instance of ChunkLayer */
    public SimpleLayer(OpenFile openFile, int left, int right, boolean opaque) {
        
    }
    
    abstract public int getLeftBound();
    abstract public int getRightBound();
    
    abstract public void insert(int offset, byte[] data);
    abstract public void delete(int offset, int count);
    abstract public void set(int offset, int count, byte[] pattern);
    
    
}
