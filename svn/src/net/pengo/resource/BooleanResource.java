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
 * BooleanResource.java
 *
 * Created on August 16, 2003, 10:50 PM
 */

package net.pengo.resource;

import java.util.List;

import net.pengo.propertyEditor.SetBooleanPage;

/**
 *
 * @author  Peter Halasz
 */
abstract public class BooleanResource extends DefinitionResource {

    public BooleanResource() {
    }

    abstract public boolean getValue();
    abstract public void setValue(boolean b);

    abstract public boolean isPrimative();
    
    /*
    public void editProperties() {
        new BooleanPrimativeResourcePropertiesForm(BooleanResource.this).show();
    }
    */
    
    public String valueDesc() {
        return getValue()+"";
    }    

    /** @return list of PropertyPage's */ 
    public List getPrimaryPages() {
        List pp = super.getPrimaryPages();
        pp.add(new SetBooleanPage(this));
        return pp;
    }    
}
