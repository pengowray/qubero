/*
 * Created on Dec 31, 2003
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.pointer;

import net.pengo.resource.Resource;

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
 
    
    //FIXME: make "object" part of param ('owner' parameter)
    //FIXME: make it: public void invoke(SmartPointer result, ListResource param){
    //FIXME: make "result" part of param too ('writable' param)
    //FIXME: add an exceptions param too 
    //FIXME: add access bits (public/protected/etc)
    //FIXME: add continuation as a param
    
    abstract public void invoke(SmartPointer result, Resource obj, Resource[] param);
    
    /** for your convenience */
    public SmartPointer invoke(Resource obj, Resource[] param){
        SmartPointer sp = new SmartPointer();
        invoke(sp, obj, param);
        return sp;
    }
    
    //FIXME: should be native types
    abstract public Class[] callSignature();
    
}
