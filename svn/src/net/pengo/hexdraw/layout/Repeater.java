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

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;

/**
 * @author Peter Halasz
 *
 */
public class Repeater extends MultiSpacer {
	private SuperSpacer contents;
	private boolean horizontal;
	
	private long maxRepeats =-1;
	
	public Repeater() {
		
	}
	
	public Repeater(SuperSpacer contents) {
		this.contents = contents;
	}
	
    public void setContents(SuperSpacer contents) {
    	this.contents = contents;
    	//recalc();
    }

	public long getSubSpacerCount() {
		//FIXME
		return 2;
	}

	public long getPixelWidth(BitCursor bits) {
		RepeatSpacerSimple main = getMainRepeater(bits); 
		BitCursor leftover = getLeftover(bits);
		
		if (isHorizontal()) {
			return main.getPixelWidth() + contents.getPixelWidth(leftover);
		} else {
			return Math.max(main.getPixelWidth(), contents.getPixelWidth(leftover));
		}
	}

	public long getPixelHeight(BitCursor bits) {
		RepeatSpacerSimple main = getMainRepeater(bits); 
		BitCursor leftover = getLeftover(bits);
		
		if (!isHorizontal()) {
			//System.out.println("main.getPixelHeight():" + main.getPixelHeight() + " contents.getPixelHeight(leftover):" + contents.getPixelHeight(leftover)
			//		+ " contents.getPixelHeight(bits): " + contents.getPixelHeight(bits));

			return main.getPixelHeight() + contents.getPixelHeight(leftover);
			
		} else {
			return Math.max(main.getPixelHeight(), contents.getPixelHeight(leftover));
		}
	}
	
	private RepeatSpacerSimple getMainRepeater(BitCursor bits) {
		//FIXME: cache return value
		BitCursor contentBits = contents.getBitCount(bits);
		
		long repeats = bits.divide(contentBits);
		//System.out.println("getMainRepeater.. h:" +isHorizontal() + " bits:" + bits + " contentBits:" + contentBits + " repeats:" + repeats + " maxReps:" + maxRepeats);
		if (maxRepeats != -1 && repeats >= maxRepeats)
			repeats = maxRepeats;
				
		RepeatSpacerSimple main = new RepeatSpacerSimple();
		main.setContents(contents);
		main.setHorizontal(isHorizontal());
		main.setContentsBits(contentBits);
		main.setRepeats(repeats);
		
		return main;
	}
	
	private BitCursor getLeftover(BitCursor bits) {
		BitCursor contentBits = contents.getBitCount(bits);

		//if more than maxRepeats, no leftovers
		long repeats = bits.divide(contentBits);
		if (maxRepeats != -1 && repeats >= maxRepeats)
			return BitCursor.zero;
		
		BitCursor leftover = bits.mod(contentBits);
		return leftover;
	}
	
	public BitCursor getBitCount(BitCursor bits) {
		if (maxRepeats == -1)
			return bits;
		
		return getMainRepeater(bits).getBitCount(bits).add(getLeftover(bits));
	}

	public BitCursor bitIsHere(long x, long y, SuperSpacer.Round r, BitCursor bits) {
		if (isHorizontal()) {
			long width = getMainRepeater(bits).getPixelWidth(bits);
			if (x >= 0 && x < width) {
				return getMainRepeater(bits).bitIsHere(x, y, r, bits);
			} else if (x >= width && x < width + contents.getPixelWidth(getLeftover(bits))) {
				BitCursor leftover = getLeftover(bits);
				if (leftover.equals(BitCursor.zero))
					return null;
				
				BitCursor location = contents.bitIsHere(x-width, y, r, leftover);
				if (location == null)
					return null;
				
				return getMainRepeater(bits).getBitCount(bits).add(location);
			}
			
		} else {
			long height = getMainRepeater(bits).getPixelHeight(bits);
			if (y >= 0 && y < height) {
				return getMainRepeater(bits).bitIsHere(x, y, r, bits);
			} else if (y >= height && y < height + contents.getPixelHeight(getLeftover(bits))) {
				BitCursor leftover = getLeftover(bits);
				if (leftover.equals(BitCursor.zero))
					return null;
				
				BitCursor location = contents.bitIsHere(x, y-height, r, leftover);
				if (location == null)
					return null;
				
				return getMainRepeater(bits).getBitCount(bits).add(location);
			}
		}
		return null;
	}

	public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel) {
		BitCursor len = seg.getLength();
		
		//FIXME: should cut short segment for main repeater? not really needed because number of repeats is worked out
		RepeatSpacerSimple main = getMainRepeater(len);
		main.paint(g,d,seg, sel);
		
		BitCursor leftover = getLeftover(len);
		if (leftover.equals(BitCursor.zero))
				return;
		
		long tranX = (isHorizontal() ? main.getPixelWidth() : 0);
		long tranY = (isHorizontal() ? 0 : main.getPixelHeight());
		g.translate((int)tranX, (int)tranY);
		
		BitSegment leftoverSeg = new BitSegment( seg.lastIndex.subtract(leftover), seg.lastIndex);

		contents.paint(g, d, leftoverSeg, sel);
		
		g.translate((int)-tranX, (int)-tranY);
	}

	public boolean isHorizontal() {
		return horizontal;
	}
	public void setHorizontal(boolean horizontal) {
		this.horizontal = horizontal;
		//recalc();
	}
	public long getMaxRepeats() {
		return maxRepeats;
	}
	public void setMaxRepeats(long maxRepeats) {
		this.maxRepeats = maxRepeats;
	}
	
	public void setSimpleSize(SimpleSize s) {
		contents.setSimpleSize(s);
	}
}
