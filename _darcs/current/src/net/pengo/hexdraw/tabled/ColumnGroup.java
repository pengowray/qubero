/**
 * A ColoumnGroup is an drawing area where _one_ style of data (eg hex) is drawn. (but many rows/columns)
 *
 * a default ColumnGroup will be made for repeating a single unit.
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.hexdraw.tabled;

import java.util.*;
//import javax.swing.table.*;

//FIXME: make this to extend Panel or something drawable
public abstract class ColumnGroup
{
	
	private float ptSize = 9; // this is the scale //FIXME:
	private boolean isPtSizeFixed = false; //setWidth (of setSize) should change the number of units OR the ptSize depending on isPtSizeFixed
	
	
	// number of hex units to draw per line. aka column count
	private int unitsPerLine;
	
	//FIXME: make default setSize method.
	//FIXME: make convinence setSize(height, units) method
	
	public void setUnitsPerLine(int unitsPerLine)
	{
		this.unitsPerLine = unitsPerLine;
	}
	

	
	public int getUnitsPerLine()
	{
		return unitsPerLine;
	}
	
	// min
	// for calculating a new size for everyone
	abstract public float widthIf(int unitsPerLine);
	
	abstract public float widthIf(int unitsPerLine, int ptSize);
	
    // is it this column group that's been selected?
    abstract public boolean getIsPrimarySelection();
    abstract public void setIsPrimarySelection(boolean primary);
	

    
	//TableColumn getColumnClass(int col);
	
    
}

