/*
 * Created on 19/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Rectangle;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.data.Data;

/**
 * @author Que
 * 
 * TODO To change the template for this generated type comment go to Window -
 * Preferences - Java - Code Style - Code Templates
 */
public class RepeatSpacerSimple extends MultiSpacer {

	private SuperSpacer contents;

	private boolean horizontal; // layout contents horizontal or
												// vertically

	private long repeats = 1; // ignores number of bits, unless
											// it's 0.

	BitCursor contentsBits = null;

	public long getSubSpacerCount() {
		// TODO Auto-generated method stub
		return 2;
	}

	private void recalc() {
		return;
	}

	private BitCursor bits() {
		return contentsBits;
	}

	/* prefer contents bits for ops on contents */
	private BitCursor bits(BitCursor inputBits) {
		if (contentsBits == null)
			return inputBits;

		return contentsBits;

	}

	public long getPixelWidth() {
		if (horizontal) {
			return contents.getPixelWidth(bits()) * repeats;
		} else {
			return contents.getPixelWidth(bits());
		}
	}

	public long getPixelWidth(BitCursor bits) {
		if (horizontal) {
			return contents.getPixelWidth(bits(bits)) * repeats;
		} else {
			return contents.getPixelWidth(bits(bits));
		}
	}

	public long getPixelHeight() {
		if (!horizontal) {
			return contents.getPixelHeight(bits()) * repeats;
		} else {
			return contents.getPixelHeight(bits());
		}
	}

	public long getPixelHeight(BitCursor bits) {
		if (!horizontal) {
			return contents.getPixelHeight(bits(bits)) * repeats;
		} else {
			return contents.getPixelHeight(bits(bits));
		}
	}

	/*
	 * (non-Javadoc)
	 * 
	 * @see net.pengo.hexdraw.layout.SuperSpacer#getBitCount(net.pengo.bitSelection.BitCursor)
	 */
	public BitCursor getBitCount(BitCursor bits) {
		if (contentsBits == null)
			return bits;

		// FIXME: multipy should take longs
		// FIXME: cache
		return bits().multiply((int) repeats);
	}

	/*
	 * (non-Javadoc)
	 * 
	 * @see net.pengo.hexdraw.layout.SuperSpacer#bitIsHere(long, long, ,
	 *         net.pengo.bitSelection.BitCursor)
	 */
	public BitCursor bitIsHere(long arg0, long arg1,
			net.pengo.hexdraw.layout.SuperSpacer.Round arg2, BitCursor arg3) {
		// TODO Auto-generated method stub
		return null;
	}

	/*
	 * (non-Javadoc)
	 * 
	 * @see net.pengo.hexdraw.layout.SuperSpacer#paint(java.awt.Graphics,
	 *         net.pengo.data.Data, net.pengo.bitSelection.BitSegment)
	 */
	public void paint(Graphics g, Data d, BitSegment seg) {
		BitCursor length = seg.getLength();
		// check for empty
		if (length.equals(BitCursor.zero))
			return;

        int totalXChange = 0;
        int totalYChange = 0;
        int startTranslateX = 0;
        int startTranslateY = 0;
        int xPixels = 0;
        int yPixels = 0;
        long first = 0;
        long reps = repeats;

        Rectangle clip = g.getClipBounds();
        if (clip.getMaxX() < 0 || clip.getMinX() > getPixelWidth(length))
        	return;
        
        if (clip.getMaxY() < 0 || clip.getMinY() > getPixelHeight(length))
        	return;
        
        if (horizontal) {
                    	
            long one = contents.getPixelWidth(length);
            xPixels = (int)one;
            
        	if (clip.getMinX() > 0) {
	            first = (long) (clip.getMinX()/one);
	            startTranslateX = (int) (one * first);
        	}
        	
            //System.out.println("first: " + first + " startX:" + startTranslateX + " clipMinX:" + clip.getMinX() + " i.lastPosCalc:" + i.lastPosCalc + " i.current:" + i.currentIndex());
            
        	if (clip.getMaxX() > 0) {
	            long last = ((long) (clip.getMaxX()/one ) +1);
	            reps = Math.min(repeats, last);
	            
        	}
            
        } else {
        	
            long one = contents.getPixelHeight(length);
            yPixels = (int)one;
            
        	if (clip.getMinY() > 0) {
	            first = (long) (clip.getMinY()/one);
	            startTranslateY = (int) (one * first);
        	}
        	
        	if (clip.getMaxY() > 0) {
	            long last = ((long) (clip.getMaxY()/one ) +1);
	            reps = Math.min(repeats, last);
	            
	            //System.out.println("Math.min(repeats, last); " + "h:" + horizontal +" repeats:" + repeats +" last:" + last);  
        	}
	            
        }
        
        g.translate(startTranslateX, startTranslateY);
        totalXChange += startTranslateX;
        totalYChange += startTranslateY;
		
        BitCursor bitLength = bits(length);
        
        BitCursor segEnd = seg.firstIndex.add(bitLength.multiply((int) first) );
        
        //System.out.println("printing " + "h:" + horizontal +  " reps:" + reps + " starting index: " + first + " pos: " + segEnd + " clip:" + clip);
		for (long i = first; i < reps; i++) {
			BitCursor segStart = segEnd;
			segEnd = segStart.add(bitLength);
			
			contents.paint(g, d, new BitSegment(segStart, segEnd));
			
			g.translate(xPixels, yPixels);
			totalXChange += xPixels; 
            totalYChange += yPixels;
		}
		
		g.translate(-totalXChange, -totalYChange);
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

	public BitCursor getContentsBits() {
		return contentsBits;
	}

	public void setContentsBits(BitCursor contentsBits) {
		this.contentsBits = contentsBits;
	}

	public long getRepeats() {
		return repeats;
	}

	public void setRepeats(long repeats) {
		this.repeats = repeats;
	}
}
