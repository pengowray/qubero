/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

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

import java.awt.Point;
import javax.swing.JPanel;
import net.pengo.app.ActiveFile;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;

/**
 *
 * @author  Peter Halasz
 */
public abstract class Spacer extends JPanel implements LongListSelectionListener {
    
    public enum Resolution { letter, word, line, paragraph, all };
    public enum Round { nearest, before, after };
    
    //fixme: create a MinorMajorSpacer as an inbetween for Spacer -> MonoSpacer
    
    private ActiveFile activeFile;
    
    private String fontName = "hex"; // to be gotten from FontMetricsCache
    private int itemSize = 12; // recommended size in px or font size. general control of size.
    private int itemCount = 16; // number of items per column. may be ignored by vertical renderers and offset columns
    private int bitCount = 8; // minor unit size how many bits each renderer recieves at a time. (must support 1-8?)
    private int majorUnitSize = 1; // selection will snap to this many minor units
    
    //private MainPanel mainPanel = null;
    //private SegmentalBitSelectionModel selection = null; // only used if no mainPanel
    
    // selection resolutions
    
    // is this spacer the one in focus (of all the spacers)
    private boolean isFocus = false;

    //too difficult, e.g. tiles may or may not be.
    //public abstract boolean isMeasuredInFontSize(); // if false, use font sizes for measurement
    
    private int orientation; // 0 horizontal, 1 vertical, fixme: make an enum
    
    

    public int getBitCount() {
        return bitCount;
    }
    
    public void setBitCount(int bitCount) {
        this.bitCount = bitCount;
    }
    
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
    
    //public abstract void paintComponent(Graphics g);
    
    public String getFontName() {
        return fontName;
    }
    
    public void setFontName(String font) {
        this.fontName = font;
    }
    
    private LongListSelectionModel selectionModel()  {
        return getActiveFile().getActive().getSelectionModel();
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
    //fixme: may also need to return screen cordinates of cursor line.
    //fixme: may also need to return screen cordinates of cursor line.
    //fixme: will need to know which sub-renderer is selected (for multi-symbols)
    public abstract BitCursor whereIsCursor(int x, int y, Resolution r);
    
    /** convert a symbol offset to a bit offset */
    /*
    public BitCursor minorSymbol2Bit(long symbolOffset) {
        //fixme: have a standard way of setting startUnit and bitoffset for whole renderer?
        int startUnit = 0;
        int unitSize = bitCount;
        int bitOffset = 0;
        long i = symbolOffset;
     
        //xxx: code from Data.readIntArray()
        long xByte = (long) (  ((startUnit + i) * unitSize + bitOffset) / 8 ); // set initial byte
        int xBit = (int) ( ((startUnit + i) * unitSize + bitOffset) % 8 ); // set initial bit
     
        return new BitCursor(xByte, xBit);
    }
     */
    
    /*
    public BitCursor majorSymbol2Bit(long symbolOffset) {
        //fixme: have a standard way of setting startUnit and bitoffset for whole renderer?
        int startUnit = 0;
        int unitSize = bitCount * ;
        int bitOffset = 0;
        long i = symbolOffset;
     
        //xxx: code from Data.readIntArray()
        long xByte = (long) (  ((startUnit + i) * unitSize + bitOffset) / 8 ); // set initial byte
        int xBit = (int) ( ((startUnit + i) * unitSize + bitOffset) % 8 ); // set initial bit
     
        return new BitCursor(xByte, xBit);
    }
     */
    
    /** Which symbol appears at this location?  */
    public abstract long whatSymbolIsHere(int x, int y);
    
    /** where is this byte on the screen?
     * may be used to line up Spacers (columns) */
    //public abstract Point offsetCoordinates(long offset);
    
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
    
    public boolean isFocus() {
        return isFocus;
    }
    
    public void setFocus(boolean isFocus) {
        this.isFocus = isFocus;
    }
    
}