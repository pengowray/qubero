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
 * DefinitionFactory.java
 *
 * Created on August 2, 2003, 11:33 PM
 */

package net.pengo.resource;

import net.pengo.app.*;
import javax.swing.*;
import java.awt.event.ActionEvent;
import java.util.Collection;
import java.util.List;
import net.pengo.restree.ResourceList;
/**
 *
 * @author  Peter Halasz
 */
abstract class DefinitionFactory extends DefinitionResource {
    List resourceList;
	
    public DefinitionFactory(List resourceList) {
		this.resourceList = resourceList;
    }
    
    public Collection getResourceList() {
		return null;
    }
    
    public ResourceList getSubResources() {
		return null;
    }
    
}
