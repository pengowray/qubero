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
public class RepeatSpacer extends MultiSpacer {

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
            BitCursor contentBits = contents.getBitCount(bits);
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
            BitCursor contentBits = contents.getBitCount(bits);
            long one = contents.getPixelHeight(contentBits);
            long allButLast = bits.divide(contentBits);
            BitCursor lastOne = bits.mod(contentBits);
            
            long length = one * allButLast + contents.getPixelHeight(lastOne);
            return length;
            
        } else {
            return contents.getPixelHeight(bits);
        }
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
                BitCursor nextBitStart = bitOffset.add( contents.getBitCount(bits) ).addBits(1); //fixme: hmm
                //System.out.println("RepeatSpacer has next if " + nextBitStart + " < " + bits + " result:" + nextBitStart.compareTo(bits));
                if (nextBitStart.compareTo(bits) >= 0) //fixme: > 0 ?
                    return false;
                
                if (bits.equals(new BitCursor())) // fixme: optimise
                	return false;
                
                return true;
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
                    
                    bitOffset = contents.getBitCount(bits).multiply((int)pos); //fixme: long->int
                    //fixme: should be more accurate for last repetition?
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
                    System.out.println("Adding: " + contents.getBitCount(bits));
                    bitOffset = bitOffset.add( contents.getBitCount(bits) );
                    next = null;
                }
                pos++;
                return contents;
            }
            
            public BitCursor getBitOffset(){
            	return bitOffset;
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
    
	public SuperSpacer getContents() {
		return contents;
	}
	public void setContents(SuperSpacer contents) {
		this.contents = contents;
	}
	public boolean isHorizontal() {
		return horizontal;
	}
	public void setHorizontal(boolean horizontal) {
		this.horizontal = horizontal;
	}

	public long getSubSpacerCount() {
		return 1;
	}

	public BitCursor getBitCount(BitCursor bits) {
		//contents.getBitCount(bits);
		//do rounding? or just use all the bits?
		return bits;

	}

	/* (non-Javadoc)
	 * @see net.pengo.hexdraw.layout.SuperSpacer#bitIsHere(long, long, , net.pengo.bitSelection.BitCursor)
	 */
	public BitCursor bitIsHere(long arg0, long arg1, net.pengo.hexdraw.layout.SuperSpacer.Round arg2, BitCursor arg3) {
		// TODO Auto-generated method stub
		return null;
	}
	

    public void paint(Graphics g, Data d, BitSegment seg) {
        SpacerIterator i;
        BitCursor length = seg.getLength();
        long last;
        
        // check for empty
        if (length.equals(new BitCursor())) // fixme: optimise
        	return;
        
        System.out.println(this.getClass().getName() + " start printing: " + seg);
        
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
        BitSegment nextSeg = new BitSegment(seg.firstIndex, seg.firstIndex.add(length));
        System.out.println("  initial:" + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
        while (i.hasNext(length) && i.currentIndex() < last) {
        	
            System.out.println("  " + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
            
            SuperSpacer sp = i.next(length);
            
            g.translate(i.getXChange(), i.getYChange());
            sp.paint(g, d, nextSeg);
            
            //fixme: check boundries
            nextSeg = new BitSegment(i.getBitOffset(), seg.lastIndex);
            
        }

        g.translate(-i.getTotalXChange(), -i.getTotalYChange());
    }
    	
}
