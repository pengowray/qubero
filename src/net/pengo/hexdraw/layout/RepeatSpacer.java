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
    public RepeatSpacerIterator iterator(long startPos, BitSegment seg) {
    	RepeatSpacerIterator si = new RepeatSpacerIterator(seg);
        si.skip(startPos);
        //si.init(rangeFirstIndex, rangeLastIndex);
        //si.init(range, startPos);
        
        return si;
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
        RepeatSpacerIterator i;
        BitCursor length = seg.getLength();

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
	            //startTranslateX = (int) (clip.getMinX() * first);
	            startTranslateX = (int) (one * first);
        	}
	            
            i = iterator(first, seg);

        	if (clip.getMaxX() > 0) {
	            i.setEndCrop((long) (clip.getMaxX()/one + 2)); // cleaner rounding up?
        	}
            
        } else {
            
        	
            long one = contents.getPixelHeight(length);
            long first = 0;
            
        	if (clip.getMinY() > 0) {
	            first = (long) clip.getMinY()/one;
	            startTranslateY = (int) (one * first);
        	}
        	
            i = iterator(first, seg);

        	if (clip.getMaxY() > 0) {
	            i.setEndCrop((long) (clip.getMaxY()/one + 2)); // cleaner rounding up?
        	}
	            
            
        }
        
        g.translate(startTranslateX, startTranslateY);
        totalXChange += startTranslateX;
        totalYChange += startTranslateY;
        
        //BitSegment nextSeg = new BitSegment(seg.firstIndex, seg.firstIndex.add(length));
        
        
//        System.out.println("  initial:" + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
//        System.out.println("  last: " + last + " X-change:" + totalXChange + " Y-change:" + totalYChange);
        
        while (i.hasNext()) {
        	
            //System.out.println("inloop: " + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
            
            SuperSpacer sp = i.next();
            
            //BitSegment currentSeg = i.currentSegment();
            
            if (i.currentIndex() >= 0) {
	            
	            sp.paint(g, d, i.currentSegment());
	            
	            if (horizontal) {
	            	long dist = contents.getPixelWidth(i.getBitCount());
	            	g.translate((int)dist, 0);
	            	totalXChange += dist;	            	
	            } else {
	            	long dist = contents.getPixelHeight(i.getBitCount());
	            	g.translate(0, (int)dist);
	            	totalYChange += dist;	            	
	            }
            
//	            g.translate(i.getXChange(), i.getYChange());
//	            totalXChange += i.getXChange();
//	            totalYChange += i.getYChange();
	          	//System.out.println("XChange:" + i.getXChange() + " YChange:" + i.getYChange());
            }
            
            //fixme: check boundries
            //nextSeg = new BitSegment(i.getBitOffset(), seg.lastIndex);

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
	
////////////////////////////////
	
	class RepeatSpacerIterator { // implements SpacerIterator
		
        long pos = -1;

        int xChange = 0;
        int yChange = 0;
        int totalXChange = 0;
        int totalYChange = 0;
        
        BitCursor bitOffset = BitCursor.zero;
        
        BitSegment range;
        //BitCursor rangeFirstIndex = BitCursor.zero;// index 0 starts at range.firstIndex. total[X,Y]change is also 0 at range.firstIndex
        //BitCursor rangeLastIndex = null;
        
        long lastPosCalc  = -1; // calculated from range
        long lastPosCrop  = -1; // given as a crop value
        
        //BitCursor lastPosCursorCalc = null;
        
        BitCursor repeatedContentBits; // bits in all but the last repetition of contents 
        BitCursor finalContentBits; // bits in the last repetition of contents
        
        Point next = null;
        BitCursor nextBitStart = null;
        
        public RepeatSpacerIterator(BitSegment range) {
        	//this.rangeFirstIndex = rangeFirstIndex;
        	//this.rangeLastIndex = rangeLastIndex;
        	this.range = range;
        	calcLastPos();
        	bitOffset = range.firstIndex;
        }
        
        public void setEndCrop(long lastPosCrop) {
        	//bitOffset = bits()
        	//do a skip.. set last
        	
        	this.lastPosCrop = lastPosCrop;
        }
        
        //public RepeatSpacerIterator() {
        	// do nothing
        	//calcLastPos();
        //}
        
        private void calcLastPos() {
        	
        	if (range == null) { // previously: if (rangeLastIndex == null) {
        		lastPosCalc = -1;
        		return; 
        	}
        	
        	//BitSegment range = new BitSegment(rangeFirstIndex, rangeLastIndex);
        	repeatedContentBits = contents.getBitCount(range.getLength());
        	finalContentBits = range.getLength().mod(repeatedContentBits);
        	
        	long lastPos = range.getLength().divide(repeatedContentBits);
        	if (! finalContentBits.equals(BitCursor.zero)) {
        		++lastPos;
        	}
        			
        	lastPosCalc = lastPos;
        }
//
//        public boolean hasNext(BitCursor bits) {
//            if (bits.equals(BitCursor.zero))
//            	return false;
//            
//            // return hasNext();
//            if (maxRepeats != -1 && pos >= maxRepeats)
//            	return false;
//            
//            //System.out.println("RepeatSpacer has next if " + nextBitStart + " < " + bits + " result:" + nextBitStart.compareTo(bits));
//            if (rangeLastIndex != null && getNextBitStart(bits).compareTo(rangeLastIndex) > 0)
//                return false;
//            
//            return true;
//        }
        
        public boolean hasNext() {
        	//maxRepeats from parent class (RepeatSpacer)
            if (maxRepeats != -1 && pos >= maxRepeats)
            	return false;
            
            if (lastPosCalc != -1 && pos >= lastPosCalc) // tho should never be -1
            	return false;
            
            if (lastPosCrop != -1 && pos >= lastPosCrop)
            	return false;
            
            return true;
        }
        
        public void skip(long n) {
        	//XXX: do this better!
        	for(long i=0; i<n; i++) {
        		next();
        	}
        }
        
        
        public BitSegment currentSegment() {
        	return new BitSegment(getBitOffset(), getBitOffset().add(bits()));
        }
        	
        
        public BitCursor getNextBitStart(BitCursor bits) {
            if (nextBitStart == null)
            	nextBitStart = bitOffset.add( contents.getBitCount(bits) ).addBits(1);
            
            return nextBitStart;
        }

        
        /* sets the cursor so that the next() will go to the startPos (nextPos) */
//        public void jumpToNext(long nextPos, BitCursor bits) {
//            pos = nextPos-1;
//            
//            if (nextPos < 0) {
//            	//System.out.println();
//            	new Exception("jump to negative pos:" + nextPos).printStackTrace();
//            }
//
//            if (nextPos > 0) {
//            	BitCursor offset = contents.getBitCount(bits).multiply((int)pos); //fixme: long->int
//                if (range != null) {
//                	bitOffset = range.firstIndex.add(offset);
//                } else {
//                	bitOffset = offset;
//                }
//            }
//            
//            //System.out.println("jumping to:" + bitOffset);
//            //fixme: should be more accurate for last repetition?
//            next = null;
//            nextBitStart = null;
//        }
        
        public SuperSpacer next() {
//			  if (pos < 0) {
//			  	//should negative pos be allowed? if so this is probably not the best way to handle it
//			  	new Exception("negative position").printStackTrace();
//			  }
			  
//		  if (horizontal) {
//		      xChange = (int)contents.getPixelWidth(bits()); // if horizontal spacer
//		      totalXChange += xChange;
//		  } else {
//		      yChange = (int)contents.getPixelHeight(bits());
//		      totalYChange += yChange;
//		  }

			  bitOffset = bitOffset.add(bits());
			  
	      	  pos++;
	      	  
			  return contents;
        }
        
        public BitCursor getBitCount() {
        	return bits();
        }
        
        //current bits
        protected BitCursor bits() {
        	return bits(pos);
        }
        
        //bits for this pos
        protected BitCursor bits(long pos) { 
        	if (pos == lastPosCalc)
        		return finalContentBits;
        	
        	if (pos > lastPosCalc || pos < 0)
        		//FIXME: give warning?
        		return BitCursor.zero;
        	
        	return repeatedContentBits;
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
        
//        public Point currentStartPoint() {
//            return new Point(xChange, yChange);
//        }
        
//        public Point nextStartPoint(BitCursor bits) {
//            if (pos < 0)
//                return new Point(0,0);
//                
//            if (next == null) {
//                if (horizontal) {
//                    next = new Point(xChange + (int)contents.getPixelWidth(bits), yChange); //fixme: warning long->int
//                } else {
//                    next = new Point(xChange, yChange + (int)contents.getPixelHeight(bits)); //fixme: warning long->int
//                }
//            }
//            
//            return next;
//        }
        
//        public int getXChange() {
//            return xChange;
//        }
//        public int getYChange() {
//            return yChange;
//        }
//        public int getTotalXChange() {
//            return totalXChange;
//        }
//        public int getTotalYChange() {
//            return totalYChange;
//        }
	}    
	
}

