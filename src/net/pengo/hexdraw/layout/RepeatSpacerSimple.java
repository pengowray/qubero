/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * Created on 19/01/2005
 *
 */
package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import java.awt.Rectangle;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;

/**
 * @author Peter Halasz
 * 
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

	 public void bitIsHere(LayoutCursorBuilder lc, Round r) {
	 	BitCursor bits = lc.getBits();
		if (isHorizontal()) {
			    //FIXME: check both y too
	    		long one = contents.getPixelWidth(bits(bits));
	            long where;
	            long x = lc.getClickX();
	            
	            if (false) {
	            	//if we're catching the event here.. FIXME: NYI	
	            	
	            	where = doRound( (float) x /one, r) ;
	            	lc.addToBlinkX(where);
	            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
	            	lc.addToBitLocation(localBitLocation);
	            	getContents().bitIsHere(lc, r);
	            	return;
	            } else {
	            	where =  x / one; // round down, let contents do real rounding
	            	long xOffset = one * where;
	            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
		            lc.addToBlinkX(xOffset);
		            lc.addToBitLocation(localBitLocation);
		            LayoutCursorBuilder view = lc.perspective((int)xOffset,0,localBitLocation);
		            view.setBits(bits(bits));
	            	getContents().bitIsHere(view,r);
		            return;
	            }
	            
    	} else {
    		long one = contents.getPixelHeight(bits(bits));
            long where;
            long y = lc.getClickY();
            if (false) {
            	//if we're catching the event here.. FIXME: NYI	
            	
            	//FIXME: copy from above
            	return;

            } else {
            	where =  y / one; // round down, let contents do real rounding
            	long yOffset = one * where;
            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
	            lc.addToBlinkY(yOffset);
	            lc.addToBitLocation(localBitLocation);

	            LayoutCursorBuilder view = lc.perspective(0,(int)yOffset,localBitLocation);
	            view.setBits(bits(bits));
	            getContents().bitIsHere(view,r);
	            return;
            
            }
    	}
		
	}
	
	public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel, LayoutCursorDescender curs, BitSegment repaintSeg) {
		BitCursor length = seg.getLength();
		// check for empty
		if (length.isZero())
			return;

        int totalXChange = 0;
        int totalYChange = 0;
        int startTranslateX = 0;
        int startTranslateY = 0;
        int xPixels = 0;
        int yPixels = 0;
        long first = 0;
        long reps = repeats;
        BitCursor bitLength = bits(length);

        Rectangle clip = g.getClipBounds();
        if (clip.getMaxX() < 0 || clip.getMinX() > getPixelWidth(length))
        	return;
        
        if (clip.getMaxY() < 0 || clip.getMinY() > getPixelHeight(length))
        	return;

        BitSegment localRepaintSeg = repaintSeg.shiftLeft(seg.firstIndex);

        if (localRepaintSeg==null)
        	return;
        
        if (repaintSeg != null) {
        	long minFirst = localRepaintSeg.firstIndex.divide(bitLength);
        	long minReps = localRepaintSeg.lastIndex.divide(bitLength) + 1;
        	
        	//System.out.println("repaint first/reps:" + minFirst + "vs" + first + ", " + repeats +"vs"+minReps);
        	
        	first = Math.max(minFirst, first);
        	repeats = Math.min(minReps, repeats);
        	//System.out.println("first/reps:" + first + ","+repeats);
        }
        
        if (horizontal) {
                    	
            long one = contents.getPixelWidth(length);
            xPixels = (int)one;
            
        	if (clip.getMinX() > 0) {
	            first = Math.max((long)(clip.getMinX()/one), first);
        	}
        	startTranslateX = (int) (one * first);
        	
            //System.out.println("first: " + first + " startX:" + startTranslateX + " clipMinX:" + clip.getMinX() + " i.lastPosCalc:" + i.lastPosCalc + " i.current:" + i.currentIndex());
            
        	if (clip.getMaxX() > 0) {
	            long last = ((long)(clip.getMaxX()/one ) +1);
	            reps = Math.min(repeats, last);
        	}
            
        } else {
        	
            long one = contents.getPixelHeight(length);
            yPixels = (int)one;
            
        	if (clip.getMinY() > 0) {
	            first = Math.max((long)(clip.getMinY()/one), first);
        	}
            startTranslateY = (int) (one * first);
        	
        	if (clip.getMaxY() > 0) {
	            long last = ((long) (clip.getMaxY()/one ) +1);
	            reps = Math.min(repeats, last);
	            //System.out.println("Math.min(repeats, last); " + "h:" + horizontal +" repeats:" + repeats +" last:" + last);  
        	}
	            
        }
        
        g.translate(startTranslateX, startTranslateY);
        totalXChange += startTranslateX;
        totalYChange += startTranslateY;
		        
        BitCursor segEnd = seg.firstIndex.add(bitLength.multiply((int) first) );
        
        //System.out.println("printing " + "h:" + horizontal +  " reps:" + reps + " starting index: " + first + " pos: " + segEnd + " clip:" + clip);
		for (long i = first; i < reps; i++) {
			BitCursor segStart = segEnd;
			segEnd = segStart.add(bitLength);
			
			contents.paint(g, d, new BitSegment(segStart, segEnd), sel, curs, repaintSeg);
			
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
	
	public void setSimpleSize(SimpleSize s) {
		contents.setSimpleSize(s);
	}

	/** adds to blinkX/Y, moves perspective to content's 0,0 */
	private LayoutCursorBuilder doRangeWork(LayoutCursorBuilder blinkLocKnown) {
		LayoutCursorBuilder lc = blinkLocKnown;
		BitCursor loc = lc.getBitLocation();
		BitCursor bitLength = lc.getBits();
		BitCursor unitBits = bits(bitLength);
		long pos = loc.divide(unitBits);
		BitCursor addition = unitBits.multiply((int)pos);
		LayoutCursorBuilder newPerspective = lc.perspective(addition);
		
		if (horizontal) {
    		long one = contents.getPixelWidth(unitBits);
    		newPerspective.addToBlinkX(one*pos);
    		newPerspective = newPerspective.perspective((int)(one*pos), 0);
		} else {
    		long one = contents.getPixelHeight(unitBits);
    		newPerspective.addToBlinkY(one*pos);
    		newPerspective = newPerspective.perspective(0, (int)(one*pos));
		}
		
		return newPerspective; 
	}

	public Point getBitRangeMin(LayoutCursorBuilder min) {
		return contents.getBitRangeMin(doRangeWork(min)); 
	}

	public Point getBitRangeMax(LayoutCursorBuilder max) {
		//System.out.println("<rep.sim> " + max);
		
		return contents.getBitRangeMax(doRangeWork(max)); 
	}

	public void updateLayoutCursor(LayoutCursorBuilder min, LayoutCursorDescender curs) {
		contents.updateLayoutCursor(doRangeWork(min), curs); 
	}	
}
