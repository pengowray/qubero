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

	/*
	 * (non-Javadoc)
	 * 
	 * @see net.pengo.hexdraw.layout.SuperSpacer#bitIsHere(long, long, ,
	 *         net.pengo.bitSelection.BitCursor)
	 */
	public LayoutCursor bitIsHere(long x, long y,
			net.pengo.hexdraw.layout.SuperSpacer.Round r, BitCursor bits, LayoutCursor lc) {
		
		if (isHorizontal()) {
			    //FIXME: check both y too
	    		long one = contents.getPixelWidth(bits(bits));
	            long where;
	            if (false) {
	            	//if we're catching the event here.. FIXME: NYI	
	            	
	            	where = doRound( (float) x /one, r) ;
	            	lc.setX(lc.getX() + where);
	            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
	            	//XXX: do we need to add it? can't we just set it?
	            	lc.setBitLocation(lc.getBitLocation().add(localBitLocation ));
	            	return lc;
	            } else {
	            	where =  x /one; // round down, let contents do real rounding
	            	long xOffset = one * where;
	            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
		            lc.setX(lc.getX()+xOffset);
	            	lc = contents.bitIsHere(x-xOffset,y,r,bits(bits),lc);
	            	lc.setBitLocation(lc.getBitLocation().add(localBitLocation));
					
		            return lc;
	            }
	            
    	} else {
    		long one = contents.getPixelHeight(bits(bits));
            long where;
            if (false) {
            	//if we're catching the event here.. FIXME: NYI	
            	
            	where = doRound( (float) y /one, r) ;
            	lc.setY(lc.getY() + where);
            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
            	//XXX: do we need to add it? can't we just set it?
            	lc.setBitLocation(lc.getBitLocation().add(localBitLocation ));
            	return lc;

            } else {
            	where =  y /one; // round down, let contents do real rounding
            	long yOffset = one * where;
            	BitCursor localBitLocation = (bits(bits)).multiply( (int) where );
	            lc.setY(lc.getY()+yOffset);
            	lc = contents.bitIsHere(x,y-yOffset,r,bits(bits),lc);
            	lc.setBitLocation(lc.getBitLocation().add(localBitLocation));
				
	            return lc;
            }
    	}
		
	}
	
	public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel, LayoutCursorDescender curs) {
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
			
			contents.paint(g, d, new BitSegment(segStart, segEnd), sel, curs);
			
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
}
