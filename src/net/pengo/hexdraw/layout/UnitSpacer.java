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
import net.pengo.data.Data;

/**
 *
 * @author  Que
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
        return tileSet.maxWidth();
    }
    
    public long getPixelHeight(BitCursor bits) {
    	return tileSet.maxHeight();
    }
    
    public BitCursor getBitCount(BitCursor bits) {
    	return bitCount;
    }
    
    public long subIsHere(int x, int y, Round round) {
		return 0;
	}
    
    public BitCursor bitIsHere(long x, long y, Round round, BitCursor bits) {
    	//fixme!!
		return null;
	}
    
    //public abstract SpacerIterator iterator(); // Iterator<SuperSpacer>
    //public abstract SpacerIterator iterator(long first);
    
    //public Point whereGoes(long sub);
    //public Point whereGoes(BitCursor bit);
    
    public void paint(Graphics g, Data d, BitSegment seg) {
    	
        if (seg.getLength().equals(new BitCursor())) // fixme: optimise
        	return;

        //System.out.println(this.getClass().getName() + " start printing: " + seg);
    	
    	try {
    		BitSegment croppedSeg = seg;
    		if (seg.getLength().compareTo(bitCount) > 0) {
    			//crop seg to bitLength
    			seg = new BitSegment(seg.firstIndex, seg.firstIndex.add(bitCount));
    		}
    		//System.out.println("old seg:" + seg + " tile.bitCount:" + bitCount + " cropped:" + croppedSeg);
    		
    		tileSet.draw(g, d.readBitsToInt(seg));
		} catch (Exception e) {
			// TODO: handle exception
			e.printStackTrace();
		}
    	
    }
    
}
