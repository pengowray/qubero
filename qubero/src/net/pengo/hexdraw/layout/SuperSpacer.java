/*
 * SuperSpacer.java
 *
 * Created on 7 December 2004, 08:24
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.data.Data;

/**
 *
 * @author  Que
 */
public abstract class SuperSpacer {
    public enum Round { nearest, before, after };
    
    /** Creates a new instance of SuperSpacer */
    public SuperSpacer() {
    }
    
    public abstract boolean isMulti();
    
    public abstract long getSubSpacerCount();
    //public abstract long getDeepSubSpacerCount();
    
    public abstract long getPixelWidth(BitCursor bits);
    public abstract long getPixelHeight(BitCursor bits);

    /** the max or default width/height */
    //public abstract long getMaxPixelWidth();
    //public abstract long getMaxPixelHeight();

    public abstract BitCursor getBitCount(BitCursor bits);
    
    //public abstract long subIsHere(int x, int y, Round round);
    public abstract BitCursor bitIsHere(long x, long y, Round round, BitCursor bits);
    
    //public abstract SpacerIterator iterator(); // Iterator<SuperSpacer>
    //public abstract SpacerIterator iterator(long first);
    
    //public abstract Point whereGoes(long sub);
    //public abstract Point whereGoes(BitCursor bit);
    
    public abstract void paint(Graphics g, Data d, BitSegment seg);
    
}