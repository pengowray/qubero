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
 * AddressPage.java
 *
 * Created on July 12, 2003, 12:34 AM
 */

package net.pengo.propertyEditor;

import net.pengo.resource.AddressedResource;
import net.pengo.resource.Resource;
import net.pengo.resource.SelectionResource;

/**
 *
 * @author  Peter Halasz
 */


public class AddressPage extends MethodSelectionPage {
    private AddressedResource res;
    
    public AddressPage(AddressedResource res){
        this(res,null);
    }
    
    public AddressPage(AddressedResource res, PropertiesForm form) {
        super(form, new PropertyPage[] {
                new SimpleAddressPage(res,form), 
                new TextOnlyPage("Multi-selection","NYI"),
                new TextOnlyPage("Empty selection","NYI"),
                new TextOnlyPage("Use another address","NYI")
        }, "Address");
        this.res = res;
        
        build();
    }

    
    public void buildOp() {
        if (res != null && modded == false) {
            SelectionResource sel = res.getSelectionResource();
            if (((Resource)res).isOwner(res.getSelectionResource())) {
	            if (sel.getSelection().getSegmentCount() > 1) {
	                setSelected(1);
	            }
	            else if (sel.getSelection().getSegmentCount() == 0) {
	                setSelected(2);
	            }
	            else {
	                setSelected(0);
	            }
            } else {
                setSelected(3);
            }
            
        }
        super.buildOp();
        
    }
    
    public String toString() {
        return "Address";
    }
    
    public void saveOp() {
        super.saveOp();
    }
}
