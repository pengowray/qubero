/*
 * columnSpacer.java
 *
 * An interface between the hexpanel and the renderers. 
 * for cleaning up data into the right bit lengths
 * and knowing where everything is.
 *
 * Only implements a single column.
 *
 * Created on 14 November 2004, 12:46
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import java.awt.event.MouseEvent;
import javax.swing.JPanel;
import net.pengo.app.ActiveFile;
import net.pengo.data.Data;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;

/**
 *
 * @author  Peter Halasz
 */
public abstract class Spacer extends JPanel implements LongListSelectionListener {
    private ActiveFile activeFile;
    
    private String fontName = "hex"; // to be gotten from FontMetricsCache
    private int itemSize = 12; // recommended size in px or font size. general control of size.
    private int itemCount = 16; // number of items per column. may be ignored by vertical renderers and offset columns
    private int bitCount = 8; // how many bits each renderer recieves at a time. (must support 1-8?)

    //too difficult, e.g. tiles may or may not be.
    //public abstract boolean isMeasuredInFontSize(); // if false, use font sizes for measurement
    
    private int orientation; // 0 horizontal, 1 vertical, fixme: make an enum
    
    /** false if proportionally spaced */
    public abstract boolean isHMonospaced();
    
    public int getOrientation() {
        return orientation;
    }
    
    public void setOrientation(int o) {
        orientation = o;
    }
    
    // usually true, false e.g. for text with random breaks
    public abstract boolean isVMonospaced();

    /** Are the symbols in the same order as the data? */
    public abstract boolean isLinear();
    
    
    public abstract int getHeight();
    public abstract int getWidth();
    
    private boolean isColumnInFocus;

    // first item of data
    private long offset; // for calculating bit position in byte stream
    private int offsetBit; // 0 to 7
    
    private int[] spaceEvery = new int[] {2, 4, 8};
    private float[] spaceSize = new float[]  {.75f, 0, .5f, 1 };
    
    /** Creates a new instance of columnSpacer */
    public Spacer() {
        
    }
    
    public abstract void paintComponent(Graphics g);
    
    public String getFontName() {
        return fontName;
    }

    public void setFontName(String font) {
        this.fontName = font;
    }
    
    public void mousePressed(MouseEvent e)  {
        if (e.getButton() == MouseEvent.BUTTON3) {
            // right click
        }
    }
    
    // MouseMotionListener
    public void mouseDragged(MouseEvent e)  {
        //Place pl =  hexFromClick( e.getX(), e.getY() );
        //if (draggingMode && pl.addr != -1)  {
            //getSelectionModel().setLeadSelectionIndex(hclick);
            //changeSelection(pl.addr, false, true);
        //}

    }
    
    /** Where is the cursor to be placed if clicked on these coordinates? 
     *
     * As a guide, the cursor should be placed to the left of a symbol if 
     * the click is on the left-HALF of the symbol, and right for
     * the right-half.
     *
     * 0 is left of first byte.
     *
     * @returns cursor is left of which byte?
     */
    public long whereIsByteCursor(int x, int y) {
        //fixme: may also need to return screen cordinates of cursor line. 
        //fixme: might not be actual bytes if using, say, 5 bit "bytes"
        return 0;
    }
    
    public long whereIsBitCursor(int x, int y) {
        //fixme: needs to return which bit        
        //fixme: may also need to return screen cordinates of cursor line.
        //fixme: will need to know which sub-renderer is selected (for multi-symbols)
        return 0;
    }

    /** Which symbol appears at this location?  */
    public abstract long whatIsHere(int x, int y);
    
    /** where is this byte on the screen?
     * may be used to line up Spacers (columns) */
    public abstract Point offsetCoordinates(long offset);

    public int getItemSize() {
        return itemSize;
    }

    public void setItemSize(int itemSize) {
        this.itemSize = itemSize;
    }

    public int getItemCount() {
        return itemCount;
    }

    public void setItemCount(int itemCount) {
        this.itemCount = itemCount;
    }

    public int getBitCount() {
        return bitCount;
    }

    public void setBitCount(int bitCount) {
        this.bitCount = bitCount;
    }
    
    public void valueChanged(LongListSelectionEvent e) {
        return;
    }

    public ActiveFile getActiveFile() {
        return activeFile;
    }

    public void setActiveFile(ActiveFile activeFile) {
        this.activeFile = activeFile;
    }

    // for your convinence
    protected Data getData() {
        return getActiveFile().getActive().getData();
    }
    
}
