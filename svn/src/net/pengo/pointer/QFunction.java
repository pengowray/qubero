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
 * Created on Dec 31, 2003
 *
 */
package net.pengo.pointer;

import net.pengo.resource.Resource;
import net.pengo.resource.SignatureTypeResource;

/**
 * @author Peter Halasz
 *
 */
abstract public class QFunction extends Resource {

    /**
     * @param openFile
     */
    public QFunction() {
        super();
    }
 
    

    
    abstract public void invoke(SmartPointer result, Resource obj, Resource[] param);
    
    /** for your convenience */
    public SmartPointer invoke(Resource obj, Resource[] param){
        SmartPointer sp = new SmartPointer();
	System.out.println("object:" + obj + ", param[0]:" + param[0]);
        invoke(sp, obj, param);
        return sp;
    }
    
    //FIXME: should be native types
    abstract public SignatureTypeResource callSignature();
    
}
