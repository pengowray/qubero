/*
 * Created on Dec 31, 2003
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.pointer;

import net.pengo.resource.Resource;
import net.pengo.resource.SignatureTypeResource;

/**
 * @author Smiley
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
