/*
 * RepeatSpacer.java
 *
 * Created on 11 December 2004, 03:32
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import java.awt.Rectangle;

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
    private int maxRepeats = -1; // -1 for infinite
    
    //fixme: ignoring these for now (always infinite)
    private BitCursor maxLength; // null for infinite ?
    
    public enum Round { nearest, before, after };
    
    public int whereIsCursor(int x, int y, BitCursor bits, Round round) {
    	return 0;
    }
    
    /** Creates a new instance of RepeatSpacer */
    public RepeatSpacer() {
    }
    
    public long getPixelWidth(BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return 0;    	
        
        if (horizontal) {
            BitCursor contentBits = contents.getBitCount(bits);
            long one = contents.getPixelWidth(contentBits);
            long allButLast = bits.divide(contentBits);
            BitCursor lastOne = bits.mod(contentBits);
            
            if (maxRepeats != -1 && allButLast > maxRepeats)
            	return maxRepeats * one;
            
            long length = one * allButLast + contents.getPixelWidth(lastOne);
            return length;
            
        } else {
            return contents.getPixelWidth(bits);
        }
    }    

    public long getPixelHeight(BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return 0;
        
        if (!horizontal) {
            BitCursor contentBits = contents.getBitCount(bits);
            long one = contents.getPixelHeight(contentBits);
            long allButLast = bits.divide(contentBits);
            BitCursor lastOne = bits.mod(contentBits);
            
            if (maxRepeats != -1 && allButLast > maxRepeats)
            	return maxRepeats * one;
            
            long length = one * allButLast + contents.getPixelHeight(lastOne);
            return length;
            
        } else {
            return contents.getPixelHeight(bits);
        }
    }    
    
    /** bits is how many bits we've got to work with total, not the size of the repeated contents. */
    public SpacerIterator iterator(long startPos, BitSegment range) {
        SpacerIterator si = iterator(range);
        si.skip(startPos);
        //si.init(range, startPos);
        return si;
    }

    public SpacerIterator iterator() {
        return new SpacerIterator() {
            long pos = -1;

            int xChange = 0;
            int yChange = 0;
            
            BitCursor bitOffset = BitCursor.zero;
            BitSegment range = null; // index 0 starts at range.firstIndex 

            Point next = null;
            BitCursor nextBitStart = null;
            
            public boolean hasNext(BitCursor bits) {
                if (bits.equals(BitCursor.zero))
                	return false;
                
                if (maxRepeats != -1 && pos >= maxRepeats)
                	return false;
                
                //System.out.println("RepeatSpacer has next if " + nextBitStart + " < " + bits + " result:" + nextBitStart.compareTo(bits));
                if (range != null && getNextBitStart(bits).compareTo(range.lastIndex) > 0)
                    return false;
                
                return true;
            }
            
            public void skip(int n, BitCursor bits) {
            	
            }
            
            /** must be called before other methods */
            public void init(BitSegment range, long startPos) {
            	this.range = range;
            	bitOffset = range.firstIndex;
            	nextBitStart = null;
            	jumpToNext(startPos, range.getLength());
            }
            
            public BitCursor getNextBitStart(BitCursor bits) {
	            if (nextBitStart == null)
	            	nextBitStart = bitOffset.add( contents.getBitCount(bits) ).addBits(1);
	            
	            return nextBitStart;
            }

            
            /* sets the cursor so that the next() will go to the startPos (nextPos) */
            public void jumpToNext(long nextPos, BitCursor bits) {
                pos = nextPos-1;
                
                if (nextPos < 0) {
                	//System.out.println();
                	new Exception("jump to negative pos:" + nextPos).printStackTrace();
                }

                if (nextPos > 0) {
                	BitCursor offset = contents.getBitCount(bits).multiply((int)pos); //fixme: long->int
	                if (range != null) {
	                	bitOffset = range.firstIndex.add(offset);
	                } else {
	                	bitOffset = offset;
	                }
                }
                
                //System.out.println("jumping to:" + bitOffset);
                //fixme: should be more accurate for last repetition?
                next = null;
                nextBitStart = null;
            }
            
            public SuperSpacer next(BitCursor bits) {
                if (pos >= 0) {
                    if (horizontal) {
                        xChange = (int)contents.getPixelWidth(bits); // if horizontal spacer
                        //totalXChange += xChange;
                    } else {
                        yChange = (int)contents.getPixelHeight(bits);
                        //totalYChange += yChange;
                    }
                    //System.out.println("Adding: " + contents.getBitCount(bits));
                    bitOffset = bitOffset.add( contents.getBitCount(bits) );
                    next = null;
                    nextBitStart = null;
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
//            public int getTotalXChange() {
//                return totalXChange;
//            }
//            public int getTotalYChange() {
//                return totalYChange;
//            }
            
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
		if (getMaxRepeats() == -1) {
			//System.out.println("getBitCount():" + bits); 
			return bits;
		}
		
		BitCursor maxBits = 
			contents.getBitCount(bits).multiply(getMaxRepeats());

		//System.out.println("getBitCount() bits:" + bits + " x maxReps:" + getMaxRepeats() + " = " + maxBits);
		
		if (bits.compareTo(maxBits) < 0)
			return bits; //do rounding?
		
		return maxBits;

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
        long last = -1;
        int totalXChange = 0;
        int totalYChange = 0;
        int startTranslateX = 0;
        int startTranslateY = 0;
        
        // check for empty
        if (length.equals(BitCursor.zero))
        	return;
        
        //System.out.println(this.getClass().getName() + " start printing: " + seg);
        
        Rectangle clip = g.getClipBounds();
        if (clip.getMaxX() < 0 || clip.getMinX() > getPixelWidth(length))
        	return;
        
        if (clip.getMaxY() < 0 || clip.getMinY() > getPixelHeight(length))
        	return;
        	
        if (horizontal) {
                    	
            long one = contents.getPixelWidth(length);
            long first = 0;
            
        	if (clip.getMinX() > 0) {
	            first = (long) clip.getMinX()/one;
	            startTranslateX = (int) (clip.getMinX() * first);
        	}
        	
        	if (clip.getMaxX() > 0) {
	            last = (long) clip.getMaxX()/one + 2; // cleaner rounding up?
        	}
	            
            i = iterator(first, seg);
            
        } else {
            
        	
            long one = contents.getPixelHeight(length);
            long first = 0;
            
        	if (clip.getMinY() > 0) {
	            first = (long) clip.getMinY()/one;
	            startTranslateY = (int) (clip.getMinY() * first);
        	}
        	
        	if (clip.getMaxY() > 0) {
	            last = (long) clip.getMaxY()/one + 2; // cleaner rounding up?
        	}
	            
            i = iterator(first, seg);
            
        }
        
        g.translate(startTranslateX, startTranslateY);
        totalXChange += startTranslateX;
        totalYChange += startTranslateY;
        
        BitSegment nextSeg = new BitSegment(seg.firstIndex, seg.firstIndex.add(length));
        System.out.println("  initial:" + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
        System.out.println("  last: " + last + " X-change:" + totalXChange + " Y-change:" + totalYChange);
        
        while (i.hasNext(length) && (last==-1 || i.currentIndex() < last) ) {
        	
            System.out.println("inloop: " + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
            
            SuperSpacer sp = i.next(length);
            
            
            
            if (i.currentIndex() >= 0) {
	            
	            sp.paint(g, d, nextSeg);
            
	            g.translate(i.getXChange(), i.getYChange());
	            totalXChange += i.getXChange();
	            totalYChange += i.getYChange();
	          	//System.out.println("XChange:" + i.getXChange() + " YChange:" + i.getYChange());
            }
            
            //fixme: check boundries
            nextSeg = new BitSegment(i.getBitOffset(), seg.lastIndex);

        }

        //System.out.println("Move back, XChange:" + i.getXChange() + " YChange:" + i.getYChange());
        g.translate(-totalXChange, -totalYChange);
    }
    	
    
    /** -1 for unlimited */
    public int getMaxRepeats() {
		return maxRepeats;
	}
	
	/** -1 for unlimited */
	public void setMaxRepeats(int maxRepeats) {
		this.maxRepeats = maxRepeats;
	}
}
