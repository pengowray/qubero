/*
 * UnitSpacer.java
 *
 * Draws a single unit.. like "FF"
 *
 * Created on 12 December 2004, 06:11
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
public class UnitSpacer extends SingleSpacer {
    
    /** Creates a new instance of UnitSpacer */
    public UnitSpacer() {
    }
    

    public abstract boolean isMulti();
    
    public abstract long getSubSpacerCount();
    public abstract long getDeepSubSpacerCount();
    
    public abstract long getPixelWidth(BitCursor bits) {
        
    }
    
    public abstract long getPixelHeight(BitCursor bits) {
        
    }
    
    public abstract BitCursor getBitCount();
    
    public abstract long subIsHere(int x, int y, Round round);
    public abstract BitCursor bitIsHere(long x, long y, Round round, BitCursor bits);
    
    public abstract SpacerIterator iterator(); // Iterator<SuperSpacer>
    //public abstract SpacerIterator iterator(long first);
    
    public abstract Point whereGoes(long sub);
    public abstract Point whereGoes(BitCursor bit);
    
    public abstract void paint(Graphics g, Data d, BitSegment seg);    
    
}
