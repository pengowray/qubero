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

/**
 * IntSelectorPage.java
 *
 * @author Peter Halasz
 */

package net.pengo.propertyEditor;

import net.pengo.resource.IntAddressedResource;

public class IntSelectorPage extends EditablePage
{
    private IntAddressedResource  res;
    
    public IntSelectorPage(IntAddressedResource res, PropertiesForm form) {
	super(form);
	this.res = res;
	
    }
    
    protected void saveOp() {
	// TODO
    }
    
    public void buildOp() {
	// TODO
    }
    
    public String toString() {
	return "Integer resource selector";
    }
}

