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
 * SuperSpacer.java
 *
 * Created on 7 December 2004, 08:24
 */

package net.pengo.hexdraw.layout;

import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Point;
import java.awt.Rectangle;
import java.util.List;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;

/**
 *
 * @author Peter Halasz
 */
public abstract class SuperSpacer {
    public enum Round { nearest, before, after };
    
    //private int simpleSize = -1; // lazily replaced with default
    
    /** Creates a new instance of SuperSpacer */
    public SuperSpacer() {
    }
    
    public abstract boolean isMulti();
    
    public abstract long getSubSpacerCount();
    //public abstract long getDeepSubSpacerCount();
    
    public abstract long getPixelWidth(BitCursor bits);
    public abstract long getPixelHeight(BitCursor bits);

    /* create a path to the clicked selection */
    public LayoutCursor layoutCursor(long x, long y, Round round, BitCursor bits) {
    	LayoutCursorBuilder lc = new LayoutCursorBuilder();
    	lc.setClickX(x);
    	lc.setClickY(y);
    	lc.setBits(bits);

    	bitIsHere(lc,round);

		return lc.toLayoutCursor();
    }
    
    /**
     * use through layoutCursor()
     * 
     * @param round
     * @param lc the path taken to reach the active selection. should isolate the column selected. mutable. add to the end of path as you go.
     */
    protected abstract void bitIsHere(LayoutCursorBuilder lc, Round round);
	
    /* add to the list with path info for this layout doodad. */
    protected List cursorPath(List path) {
    	// return contents.cursorPath(path);
    	return path;
    }
    
    /** the max or default width/height */
    //public abstract long getMaxPixelWidth();
    //public abstract long getMaxPixelHeight();

    public abstract BitCursor getBitCount(BitCursor bits);
    
    //public abstract long subIsHere(int x, int y, Round round);

	public Rectangle getBitRangeRectangle(BitSegment range, BitCursor bits) {
		LayoutCursorBuilder min = new LayoutCursorBuilder();
		min.setBits(bits);
		min.setBitLocation(range.firstIndex);
		
		LayoutCursorBuilder max = new LayoutCursorBuilder();
		max.setBits(bits);
		max.setBitLocation(range.lastIndex);
		
		return getBitRangeRectangle(min, max);
	}

	/** does not work. */
	public Rectangle getBitRangeRectangle(LayoutCursorBuilder min, LayoutCursorBuilder max) {
		/* this is flawed, as the start and end tiles do not have the properties of having the extreme min and max values. */ 
		Point topLeft  = getBitRangeMin(min);
		Point botRight = getBitRangeMax(max);
		Dimension relativeBotRight = new Dimension(botRight.x - topLeft.x, botRight.y - topLeft.y );
		return new Rectangle(topLeft, relativeBotRight);
	}
	
	public abstract Point getBitRangeMin(LayoutCursorBuilder min);
	public abstract Point getBitRangeMax(LayoutCursorBuilder max);

	public abstract void updateLayoutCursor(LayoutCursorBuilder lc, LayoutCursorDescender curs);
	
    public abstract void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel, LayoutCursorDescender curs, BitSegment repaintSeg);
    
	public abstract void setSimpleSize(SimpleSize s);
	
	protected static long doRound(double f, net.pengo.hexdraw.layout.SuperSpacer.Round r) {
		if (r == Round.before) {
			return (long)Math.floor(f);
		} else if (r == Round.after) {
			return (long)Math.ceil(f);
		}  
		
		// (r == Round.nearest)
		return (long)Math.round(f);		
		
	}

}
