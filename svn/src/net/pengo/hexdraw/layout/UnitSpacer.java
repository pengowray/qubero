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
 * UnitSpacer.java
 *
 * Draws a single unit.. like "FF"
 *
 * Created on 12 December 2004, 06:11
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;

/**
 *
 * @author Peter Halasz
 */
public class UnitSpacer extends SingleSpacer {
    
	TileSet tileSet;
	BitCursor bitCount;
	
    /** Creates a new instance of UnitSpacer */
    public UnitSpacer(TileSet tileSet) {
    	this.tileSet = tileSet;
    	this.bitCount = BitCursor.newFromBits( tileSet.getBitsPerTile() );
    	
    }

    public UnitSpacer(TileSet tileSet, BitCursor bitCount) {
    	this.tileSet = tileSet;
    	this.bitCount = bitCount; 
    }
    
    public long getPixelWidth(BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return 0;    	

    	
        return getMaxPixelWidth();
    }
    
    public long getPixelHeight(BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return 0;    	
    	
    	return getMaxPixelHeight();
    }

    public long getMaxPixelWidth() {
    	return tileSet.maxWidth();
    }
    
    public long getMaxPixelHeight() {
    	return tileSet.maxHeight();
    }
    
    public BitCursor getBitCount(BitCursor bits) {
    	return bitCount;
    }
    
    public long subIsHere(int x, int y, Round round) {
		return 0;
	}
    
    public BitCursor bitIsHere(long x, long y, Round round, BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return BitCursor.zero;
    	
    	if (round == Round.before)
    		return BitCursor.zero;
    	
    	else if (round == Round.after)
    		return bitCount;
    	
		else {
			//Round.nearest
			if (x <= getPixelWidth(bits)/2) {
				return BitCursor.zero;
			} else {
				return bitCount;
			}
		}
    }
    
    //public abstract SpacerIterator iterator(); // Iterator<SuperSpacer>
    //public abstract SpacerIterator iterator(long first);
    
    //public Point whereGoes(long sub);
    //public Point whereGoes(BitCursor bit);
    
    public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel) {

    	//System.out.print('/');
        if (seg.getLength().equals(new BitCursor())) // fixme: optimise
        	return;

        //System.out.println(this.getClass().getName() + " start printing: " + seg);
    	
    	try {
    		BitSegment croppedSeg = seg;
    		if (seg.getLength().compareTo(bitCount) > 0) {
    			//crop seg to bitLength
    			seg = new BitSegment(seg.firstIndex, seg.firstIndex.add(bitCount));
    		//System.out.println("old seg:" + seg + " tile.bitCount:" + bitCount + " cropped:" + croppedSeg);
    		}
    		
    		int redbits = d.readBitsToInt(seg);
    		//if (redbits < 0)
    		//	System.out.println("negative!");a
    		//boolean selected = sel.isSelectedIndex(seg.firstIndex) && sel.isSelectedIndex(seg.lastIndex.subtract(BitCursor.oneBit));
    		boolean selected = sel.isSelectedIndex(seg.firstIndex) && sel.isSelectedIndex(seg.lastIndex);
    		
    		if (selected) {
    			g.drawRect(0,0,(int)getPixelWidth(seg.getLength()), (int)getPixelHeight(seg.getLength()));
			}
    		
    		tileSet.draw(g, redbits);
		} catch (Exception e) {
			// TODO: handle exception
			e.printStackTrace();
		}
		//System.out.print('/');
    	
    }
    
	public void setSimpleSize(SimpleSize s) {
		tileSet.setSimpleSize(s);
	}	    
}
