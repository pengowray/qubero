/*
 * columnSpacer.java
 *
 * An interface between the hexpanel and the renderers. 
 * for cleaning up data into the right bit lengths
 * and knowing where everything is
 *
 * Created on 14 November 2004, 12:46
 */

package net.pengo.hexdraw.layout;

import net.pengo.hexdraw.original.Place;

/**
 *
 * @author  Peter Halasz
 */
public abstract class Spacer {
    private String fontName; // to be gotten from FontMetricsCache
    private int itemSize; // recommended size in px or font size. general control of size.
    public int itemCount; // number of items per column. may be ignored by vertical renderers and offset columns
    private boolean isColumnInFocus;
    private int bitCount = 8; // how many bits each renderer recieves at a time. must support 1-8.
    public abstract int getHeight();
    public abstract int getWidth();
    
    public abstract boolean isMeasuredInFontSize(); // if false, use font sizes for measurement
    public abstract boolean isMonospaced(); // false if proportional or vertical
    public abstract boolean isVerticallyMonospaced(); // usually true, false e.g. for text with random breaks
    
    public abstract Place whereAmI(int x, int y);

    // first item of data
    private long offset; // for calculating bit position in byte stream
    private int offsetBit; // 0 to 7
    
    
    private int[] spaceEvery = new int[] {2, 4, 8};
    private float[] spaceSize = new float[]  {.75f, 0, .5f, 1 };
    
    
    /** Creates a new instance of columnSpacer */
    public Spacer() {
        
    }
    
    public String getFontName() {
        return fontName;
    }

    public void setFontName(String font) {
        this.fontName = font;
    }
}
