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
import net.pengo.splash.SimpleSize;

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
        
        /*
        System.out.println("Creating iterator: " + seg);
        debugPrint(new RepeatSpacerIterator(seg));
        RepeatSpacerIterator test = new RepeatSpacerIterator(seg);
        test.skip(startPos);
        if (startPos != 0) {
	        System.out.println("Again, skipping to: " + startPos);
	        debugPrint(test);
        }
        */
        
        return si;
    }
    
    private void debugPrint(RepeatSpacerIterator i){
    	while (i.hasNext()) {
    		SuperSpacer ss = i.next();
    		System.out.println("i:" + i.currentIndex() + " seg:" +i.currentSegment() +" count:" + i.getBitCount() );
    	}
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
    	
    	System.out.println(seg + " being painted..");
		
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
	            first = (long) (clip.getMinX()/one);
	            //startTranslateX = (int) (clip.getMinX() * first);
	            startTranslateX = (int) (one * first);
	            
        	} else {
        		//	all ok.
        	}
	            
            i = iterator(first, seg);
            
            System.out.println("first: " + first + " startX:" + startTranslateX + " clipMinX:" + clip.getMinX() + " i.lastPosCalc:" + i.lastPosCalc + " i.current:" + i.currentIndex());
            
        	if (clip.getMaxX() > 0) {
	            i.setEndCrop((long) (clip.getMaxX()/one ) +1); // +1 cleaner rounding up?
        	}
            
        } else {
        	
            long one = contents.getPixelHeight(length);
            long first = 0;
            
        	if (clip.getMinY() > 0) {
	            first = (long) (clip.getMinY()/one);
	            startTranslateY = (int) (one * first);
        	}
        	
        	i = iterator(first, seg);

        	if (clip.getMaxY() > 0) {
	            i.setEndCrop((long) (clip.getMaxY()/one ) +1); // +1 ? cleaner rounding up?
        	}
	            
            
        }
        
        g.translate(startTranslateX, startTranslateY);
        totalXChange += startTranslateX;
        totalYChange += startTranslateY;
        
        //BitSegment nextSeg = new BitSegment(seg.firstIndex, seg.firstIndex.add(length));
        
        
//        System.out.println("  initial:" + nextSeg + " length:" + length + " index:" + i.currentIndex() + "/" + last);
//        System.out.println("  last: " + last + " X-change:" + totalXChange + " Y-change:" + totalYChange);
        //System.out.println("---");
        
        while (i.hasNext()) {
        	//System.out.print(".");
            //System.out.println(" index:" + i.currentIndex() + "of ?, current segment:" + i.currentSegment() ); // + "/" + last
            
            SuperSpacer sp = i.next();
            
            //BitSegment currentSeg = i.currentSegment();
            
            if (i.currentIndex() >= 0) {
	            sp.paint(g, d, i.currentSegment());
	            
	            if (horizontal) {
	            	//System.out.print("h");
	            	long dist = contents.getPixelWidth(i.getBitCount());
	            	g.translate((int)dist, 0);
	            	totalXChange += dist;	            	
	            } else {
	            	//System.out.print("v");
	            	long dist = contents.getPixelHeight(i.getBitCount());
	            	g.translate(0, (int)dist);
	            	totalYChange += dist;
	            }
            
//	            g.translate(i.getXChange(), i.getYChange());
//	            totalXChange += i.getXChange();
//	            totalYChange += i.getYChange();
	          	//System.out.println("XChange:" + contents.getPixelWidth(i.getBitCount()) + " YChange:" + contents.getPixelHeight(i.getBitCount()));
            } else {
            	//ERROR SHOULD NOT OCCUR
            	System.out.print("error: -1-1-1-1-1-1-1-1-1-1-1-1-1-1-1-1-1-1-1");
            }
            
            //fixme: check boundries
            //nextSeg = new BitSegment(i.getBitOffset(), seg.lastIndex);

        }
        
        //System.out.println("---");

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
        	
        	long lastPos = range.getLength().divide(repeatedContentBits) -1;
        	if (! finalContentBits.equals(BitCursor.zero)) {
        		//System.out.println("***extra bit on " + range + " len:" + finalContentBits + " vs:" + repeatedContentBits);
        		++lastPos;
        	} else {
        		finalContentBits = repeatedContentBits;
        	}
        	
        	System.out.println("lastPos:" + lastPos + " for range:" + range);
        
        	lastPosCalc = lastPos;
        }
        
        public boolean hasNext() {
        	//maxRepeats from parent class (RepeatSpacer)
            if (maxRepeats != -1 && pos >= maxRepeats) {
            	System.out.println("max rep");
            	return false;
            }
            
            if (lastPosCalc != -1 && pos >= lastPosCalc) { // tho should never be -1 
            	System.out.println("max pos");
            	return false;
            }
            
            if (lastPosCrop != -1 && pos >= lastPosCrop) {
            	System.out.println("max crop");
            	return false;
            }
            
            return true;
        }
        
        public BitSegment currentSegment() {
        	return new BitSegment(getBitOffset(), getBitOffset().add(bits()));
        }        	

        public void skip(long n) {
        	//XXX: do this better!

        	if (n<0)
        		new Exception("negative skip!").printStackTrace();
        	
        	if (n==0) //FIXME: this happens too often, fix calling code
        		return; 
        	
        	if (pos == -1) {
        		n--;
        		if (n==0)
            		return;
        	}
        	
        	
        	
        	BitCursor addbits = repeatedContentBits.multiply( (int)n );
        	
			bitOffset = bitOffset.add( addbits );
			  
	      	// System.out.println("Skipping:" + n + " bits-per-step:" + bits(1) + " bits:" + addbits + " old pos:" + pos + " new pos:" + (pos+n));

	      	pos+=n;
        }
        
        public SuperSpacer next() {
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
        	if (pos < 0) {
        		return BitCursor.zero;
        	}

        	if (pos == lastPosCalc) {
        		//System.out.println("--------test: " + finalContentBits + " pos:" + pos + " range:" + range);
        		return finalContentBits;
        	}
        	
        	if (pos > lastPosCalc) {
        		//FIXME: give warning?
        		System.out.println("00000----test==" + repeatedContentBits);
        		return BitCursor.zero;
        	}
        	
        	//System.out.println("+++++----test==" + repeatedContentBits);
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
        
	}

	/* (non-Javadoc)
	 * @see net.pengo.hexdraw.layout.SuperSpacer#setSimpleSize(net.pengo.splash.SimpleSize)
	 */
	public void setSimpleSize(SimpleSize s) {
		// TODO Auto-generated method stub
		
	}    
	
}

