/*
 * RepeatSpacer.java
 *
 * Created on 11 December 2004, 03:32
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
public class RepeatSpacer {

    private SuperSpacer contents;
    private boolean horizontal; //  layout contents horizontal or vertically 
    
    //fixme: ignoring these for now (always infinite)
    private BitCursor maxLength; // null for infinite ?
    private int maxRepeats = -1; // -1 for infinite
    
    /** Creates a new instance of RepeatSpacer */
    public RepeatSpacer() {
    }
    
    public long getPixelWidth(BitCursor bits) {
        
        if (horizontal) {
            BitCursor contentBits = contents.getBitCount();
            long one = contents.getPixelWidth(contentBits);
            long allButLast = bits.divide(contentBits);
            BitCursor lastOne = bits.mod(contentBits);
            
            long length = one * allButLast + contents.getPixelWidth(lastOne);
            return length;
            
        } else {
            return contents.getPixelWidth(bits);
        }
    }    

    public long getPixelHeight(BitCursor bits) {
        
        if (!horizontal) {
            BitCursor contentBits = contents.getBitCount();
            long one = contents.getPixelHeight(contentBits);
            long allButLast = bits.divide(contentBits);
            BitCursor lastOne = bits.mod(contentBits);
            
            long length = one * allButLast + contents.getPixelHeight(lastOne);
            return length;
            
        } else {
            return contents.getPixelHeight(bits);
        }
    }    
    
    

    public void paint(Graphics g, Data d, BitSegment seg) {
        SpacerIterator i;
        BitCursor length = seg.getLength();
        long last;
        
        if (horizontal) {
            
            long clipStart = (long)g.getClipBounds().getMinX();
            long clipEnd = (long)g.getClipBounds().getMaxX();
            long one = contents.getPixelWidth(length);
            
            //get first repetition within clip and number of reps needed
            long first = clipStart/one;
            last = clipEnd/one; //fixme: round up? don't think need to.
            
            i = iterator(first, length);
            
            //contents.getBitCount();
            
        } else {
            
            long clipStart = (long)g.getClipBounds().getMinY();
            long clipEnd = (long)g.getClipBounds().getMaxY();
            long one = contents.getPixelHeight(length);
            
            //get first repetition within clip and number of reps needed
            long first = clipStart/one;
            last = clipEnd/one; //fixme: round up? don't think need to.
            
            i = iterator(first, length);
            
        }
        
        g.translate(i.getTotalXChange(), i.getTotalYChange());
        while (i.hasNext(length) && i.currentIndex() < last) {
            SuperSpacer sp = i.next(length);
            g.translate(i.getXChange(), i.getYChange());
            sp.paint(g, d, seg);
        }

        g.translate(-i.getTotalXChange(), -i.getTotalYChange());
    }
    
    
    /** bits is how many bits we've got to work with total, not the size of the repeated contents. */
    public SpacerIterator iterator(long startPos, BitCursor bits) {
        SpacerIterator si = iterator();
        si.jumpToNext(startPos, bits);
        return si;
    }

    public SpacerIterator iterator() {
        return new SpacerIterator() {
            long pos = -1;

            int xChange = 0;
            int yChange = 0;
            int totalXChange = 0;
            int totalYChange = 0;
            
            BitCursor bitOffset = new BitCursor();
            
            Point next = null;
            
            public boolean hasNext(BitCursor bits) {
                BitCursor nextBitStart = bitOffset.add( contents.getBitCount() );
                if (nextBitStart.compareTo(bits) < 0) //fixme: <= 0 ?
                    return true;
                
                return false;
            }
            
            /* sets the cursor so that the next() will go to the startPos */
            public void jumpToNext(long nextPos, BitCursor bits) {
                pos = nextPos-1;
                
                if (pos == 0) {
                    //fixme: can scrap this exception and call jumpToNext(0) whenever creating an iterator..?
                    totalXChange = 0;
                    totalYChange = 0;
                } else {
                    
                    if (horizontal) {
                        totalXChange = (int) (contents.getPixelWidth(bits) * pos);
                    } else {
                        totalYChange = (int) (contents.getPixelHeight(bits) * pos);
                    }
                }
                next = null;
                
            }
            
            public SuperSpacer next(BitCursor bits) {
                if (pos >= 0) {
                    if (horizontal) {
                        xChange = (int)contents.getPixelWidth(bits); // if horizontal spacer
                        totalXChange += xChange;
                    } else {
                        yChange = (int)contents.getPixelHeight(bits);
                        totalYChange += yChange;
                    }
                    
                    bitOffset = bitOffset.add( contents.getBitCount() );
                    next = null;
                }
                
                return contents;
            }
            
            public void remove() {
                //fixme nyi. throw exception?
            }

            public SuperSpacer currentSpacer() {
                return contents;
            }
            
            public long currentIndex() {
                return pos;
            }
            
            public Point currentStartPoint() {
                return new Point(xChange, yChange);
            }
            
            public Point nextStartPoint(BitCursor bits) {
                if (pos < 0)
                    return new Point(0,0);
                    
                if (next == null) {
                    if (horizontal) {
                        next = new Point(xChange + (int)contents.getPixelWidth(bits), yChange); //fixme: warning long->int
                    } else {
                        next = new Point(xChange, yChange + (int)contents.getPixelHeight(bits)); //fixme: warning long->int
                    }
                }
                
                return next;
            }
            
            public int getXChange() {
                return xChange;
            }
            public int getYChange() {
                return yChange;
            }
            public int getTotalXChange() {
                return totalXChange;
            }
            public int getTotalYChange() {
                return totalYChange;
            }
            
        };
    }    
    
}
