/*
 * DirectionalSegment.java
 *
 * Created on 28 November 2004, 23:16
 */

package net.pengo.bitSelection;

/**
 *
 * @author  Que
 */
public class DirectionalSegment extends BitSegment {
    private boolean isFacingRight;
    
    /** Creates a new instance of DirectionalSegment */
    public DirectionalSegment(long head, long tail) {
        this(new BitCursor(head,0), new BitCursor(tail,0));
    }
    
    public DirectionalSegment(BitCursor head, BitCursor tail) {
        super(head,tail);
        if (head == lastIndex) {
            isFacingRight = true;
        } else {
            isFacingRight = false;
        }
    }
    
    // or lead
    public BitCursor getHead() {
        if (isFacingRight) {
            return lastIndex;
        } else {
            return firstIndex;
        }
    }

    // or anchor
    public BitCursor getTail() {
        if (!isFacingRight) {
            return lastIndex;
        } else {
            return firstIndex;
        }
    }
    
    //fixme: for completeness, override subtract method (from BitSegment) to preserve direction
}
