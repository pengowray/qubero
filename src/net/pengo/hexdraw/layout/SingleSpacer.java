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
 * SingleSpacer.java
 *
 * Created on 7 December 2004, 08:25
 */

package net.pengo.hexdraw.layout;

import java.awt.Point;
import java.awt.Rectangle;

import net.pengo.bitSelection.BitCursor;

/**
 *
 * @author Peter Halasz
 */
public abstract class SingleSpacer extends SuperSpacer {
    
    /** Creates a new instance of SingleSpacer */
    public SingleSpacer() {
    }
    
    public boolean isMulti() {
        return false;
    }
    
    public long getSubSpacerCount() {
        return 1;
    }
    
	public Point getBitRangeMin(LayoutCursorBuilder min) {
		return min.restorePerspective().getBlinkPoint();
	}

	public Point getBitRangeMax(LayoutCursorBuilder max) {
		BitCursor bits = max.getBits(); // must do before restoring
		max.addToBlinkX(getPixelWidth(bits));
		max.addToBlinkY(getPixelHeight(bits));
		
		return max.restorePerspective().getBlinkPoint();
	}
	
    protected void bitIsHere(LayoutCursorBuilder lc, Round round) {
    	BitCursor bits = getBitCount(lc.getBits());
    	

    	if (bits.isZero()) {
    		//lc.setNull(true);
    		return;
    	}
    	
    	if (round == Round.before) {
    		calcBlinky(lc, bits);
    		return;
    	
    	} else if (round == Round.after) {
    		lc.addToBitLocation(bits);
//    		lc.addToBlinkX(getMaxPixelWidth()); // from unitSpacer
    		lc.addToBlinkX(getPixelWidth(bits));
    		calcBlinky(lc, bits);
    		return;
    	
    	} else {
			assert round == Round.nearest;
			if (lc.getClickX() <= getPixelWidth(bits)/2) {
	    		calcBlinky(lc, bits);
				return;
			} else {
	    		lc.addToBitLocation(getBitCount(lc.getBits()));
//	    		lc.addToBlinkX(getMaxPixelWidth()); // from unitSpacer
	    		lc.addToBlinkX(getPixelWidth(bits));
	    		calcBlinky(lc, bits);
	    		return;
			}
		}
    }

    protected void calcBlinky(LayoutCursorBuilder lc, BitCursor gottenBits) {
    	lc = lc.restorePerspective();
    	//lc.setBlinkyLocation(new Rectangle((int)lc.getBlinkX(), (int)lc.getBlinkY(), 1, (int)getMaxPixelHeight()));
    	lc.setBlinkyLocation(new Rectangle((int)lc.getBlinkX(), (int)lc.getBlinkY(), 1, (int)getPixelHeight(gottenBits)));
    }
}
