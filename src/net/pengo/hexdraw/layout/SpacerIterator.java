/*
 * SpacerIterator.java
 *
 * Created on 10 December 2004, 07:02
 */

package net.pengo.hexdraw.layout;

import java.awt.Point;

import net.pengo.bitSelection.BitCursor;

/**
 *
 * @author  Que
 */
public interface SpacerIterator  { // extends Iterator<SuperSpacer>

    boolean hasNext(BitCursor bits);
    SuperSpacer next(BitCursor bits);
    void remove();
    
    SuperSpacer currentSpacer();
    long currentIndex();
    Point currentStartPoint();
    Point nextStartPoint(BitCursor bits);

    public int getXChange();
    public int getYChange();
    public int getTotalXChange();
    public int getTotalYChange();
    
    public void jumpToNext(long nextPos, BitCursor bits);
    
    public BitCursor getBitOffset();    
}
