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
 * Created on 23/01/2005
 */
package net.pengo.hexdraw.layout;

import java.util.ArrayList;

/**
 * @author Peter Halasz
 */
public class Columns {
    private ArrayList<SuperSpacer> columns = new ArrayList<SuperSpacer>();
    
    public void addColumn(SuperSpacer unit, int repeats) {
    	Repeater row = new Repeater();
        row.setHorizontal(true);
        row.setContents(unit);
        row.setMaxRepeats(repeats);
        
        Repeater column = new Repeater();
        column.setHorizontal(false);
        column.setContents(row);
        
    	columns.add(column);
    }
    
    public SuperSpacer[] toArray() {
    	return columns.toArray(new SuperSpacer[0]); 
    }
    
    public void clear() {
    	columns.clear();
    }

    
    public GroupSpacer toColumnGroup() {
	   	GroupSpacer page = new GroupSpacer();
		page.setContents(toArray());
		page.setHorizontal(true);
		
		return page;
    }
}
